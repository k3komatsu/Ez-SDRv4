# 1b-4c：Sol 6.1 の再レビュー（修正の回 2・3・4 の後）

原文の作業ディレクトリ：`~/.cache/ezsdr-fixes/2026-10-19-phase8-1b4c/`（`sol-rereview{,2,3}.md`．下はそれぞれの判定から後）．

## 再レビュー 1

判定：FAIL

1. **major — 拒否された TX switch が、未検証の channel 集合を「開いている集合」として残す**
   
   **場所:** `crates/ezsdr-radio-uhd/src/provider/tx.rs:179,670-695`、`crates/ezsdr-radio-uhd/src/provider/control.rs:793,829`
   
   **根拠:** from-none の失敗も `configured = true` で登録される。owner がその switch を読んで実行する前は `idle = false`。そこで次の cold 更新を取ると、二つ目は `configured = false` になる。最初の switch は open・照合を飛ばす一方、拒否の分岐より前に `self.channels = to.set()` を実行する。二つ目の集合が同じなら、必要な open／rate 照合を飛ばして configure する。
   
   **失敗する列:**
   - TX が空 → from-none の open が失敗 → owner が `read(None)` → 同じ集合を使う rate 更新 → 二つの switch を実行。
   - 二つ目は streamer が無いまま configure・clock 開始へ進む。`UhdDevice` は失敗した reopen の前に旧 streamer を除去するため、後の送信が失敗する（`crates/ezsdr-radio-uhd/src/uhd.rs:1011-1020,1025-1026`）。
   - **open 成功・rate 照合拒否でも同じ問題になる。** 二つ目が保存済みの拒否を再適用せず、未対応の rate 集合を持つ device で再開できる。
   
   報告された欠陥は確認できた。「決定的に作れない」という判断には同意しない。既存の TX unit test と同様、`read` と `step` を直接駆動すれば競合の窓を固定できる。
   
   **修正:** `self.channels` の更新を成功時へ移し、拒否・open／照合失敗時は opened-set を無効化する。単に「open が成功した集合」だけを保存する修正では、rate 照合拒否のケースを救えない。
   
   **テスト:** 一つ目の拒否後に `tx.read(None)` だけ実行し、`step` 前に同じ集合の rate 更新を記帳する。その後 switch を進め、open 失敗からは再open・初回照合が必要なこと、照合拒否からは保存済み拒否が再適用され、clock／burst が開始されないことを固定する。対応する mutation も必要。**実行再現は未実施（INFERRED）；分岐と状態遷移はコードで確認。**

### 項目別確認

- **1 — 大修正:** O1 の単一 critical section、lock を取らない `ColdConfig` 構築、TX report の位置による相関、refused item による値保持・halt、拒否 TX clock の不開始は妥当。LockSpy と P271・P349〜P359 の対応も確認。残る問題は上記1。
- **2 — 隣接列:** 拒否後の timed 更新、RX の再開、Stop、refused clock への burst admission を追跡。上記1以外の追加欠陥は見つからなかった。`updates → streams → rec` と、device 呼び出し中の line lock 解放も妥当。
- **3 — 初回 open と未修正欠陥:** `Control::new` が owner 起動前に `idle` を設定する反論は正しい（`provider/mod.rs:662-668`）。以前の私の初期値に基づく推論は撤回する。RX 初回照合・保存結果の再適用・TX 再適用の置換テストは妥当。ただし TX の pending-refusal 列は未固定で、上記1が残る。
- **4 — minor 修正:** LATE→拒否の順序、方向間 floor と clock cut、論理更新比較、M1 の channel 1 loopback、宣言済み rate key、spec 修正、冗長検査と R23 の削除を確認。採らなかった `note` 除外・200-tick 許容差・既存の Phase 8 文言に追加異議はない。
- **5 — mutation:** 両リスト全909行の `old` が各1回一致。HEAD 比で変更・追加された114行の差し替えと filter の存在を確認。P313 は後続 stream に line がなく、coerce 後の rate が格子上なので同値という説明が成立。G03・J58・J156 にも今回差分による反例は見つからなかった。**kill 成績は再実行しておらず、実際の kill は INFERRED。**

## 再レビュー 2

判定：FAIL

独自に確認した故障系列：失敗後の同集合・別集合への再開、別集合を開いた後の旧集合への復帰、owner が読む前に中間 segment が消える場合、Stop・prune を挟む場合。

## 指摘

1. **major — RX は control 側の open 成功後に古い集合を再利用できる**
   - **場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:808`、同 `:825–833`、`crates/ezsdr-radio-uhd/src/provider/rx.rs:283–299`
   - **根拠：** `unopened` が立つのは open／rate 照合の失敗だけ。別集合への open が成功して configure が失敗した場合、device の集合は変わるが、owner の `opened` は旧集合のまま残る。RX はゼロ長 segment を計画から除くため、その間にある segment が owner の記録を更新する保証もない（`crates/ezsdr-radio/src/timeline.rs:913–918`）。
   - **失敗系列：** 集合 A=`{0}` の受信を停止し、owner を idle にする → control が集合 B=`{0,1}` を開き、照合成功後に configure だけ失敗 → owner を進めず `start_rx` を記帳 → 続けて同じ実効時刻で集合 A への cold 更新を記帳する。Start の B は start lead より前で切られて省略され、A の更新は pending segment があったため owner 担当になる。`begin()` は `opened == A` により reopen を省き、実際には B の streamer を A として開始する。**この操作列全体の実行再現は未実施：INFERRED。**
   - **成功時にも同型：** control の B の configure まで成功しても、owner が読む前に A の cold 更新が B をゼロ長にすれば、同じ不整合が起こる。
   - **修正：** control が RX の open を試みた時点で、成否にかかわらず owner の集合キャッシュを無効化する。次が `configured` の成功 segment なら既存の経路で集合を更新できるため、余計な reopen は不要。この二系列を、owner を停止した決定的なテストで固定する。

## 項目別確認

1. **TX：問題なし。** 拒否・open／照合／configure 失敗で `verified` が落ち、同集合・旧集合への復帰でも再 open・保存済み照合が必要になる。拒否時は clock を採用しない（`tx.rs:676–703`）。
2. **RX：未完了。** open／照合失敗の印は stream ごとで、prune・Stop では消えず、`begin` が owner の空集合へ引き継ぐ。device 呼び出し中の line lock 保持もない。ただし上記の configure 失敗・成功 segment 省略が漏れている。
3. **非対称性：通知経路は必要。** RX は拒否・ゼロ長 segment を見ないことがあり、control→owner の無効化通知が必要。共通の仕組みへ大改修するより、RX の印を「control が open を試みた」に広げる方が小さく正しい。
4. **六テストと行：局所的には妥当。** 六本とも実行成功。P361–P366 と P318 は各対象にちょうど一回一致し、置換は対応する assertion を壊す内容。ただし上記系列は観測していない。mutation の実際の kill は再実行していないため **INFERRED**。
5. **既存性質：確認範囲で維持。** lib 全131本（rm_26 両差分試験を含む）、fake の `ur_13_` 全12本、P358 の対象、拒否時の値保持・halt の対象は成功した。

未確認：実機・mutation・workspace 全体。fake の `ur_25_` 一括実行は120秒で打ち切られたが、停止位置の値保持テストは単独再実行で成功した。

## 再レビュー 3

判定：PASS

修正4で前回の RX 指摘は閉じています。P2-3 は実在する既存欠陥ですが、**1b-4d の最初に直す既知の major として、今回からの延期を認めます**。既存だから無害、という判断ではありません。

## 1. 前回指摘の閉鎖確認

- `crates/ezsdr-radio-uhd/src/provider/control.rs:797–835`：from-none の RX open を試みた場合、open・照合・configure の結果によらず `stale` を保存します。
- `crates/ezsdr-radio-uhd/src/provider/rx.rs:277–300`：次の計画の取得と同じ lock 内で印を消費し、owner の `opened` を空にします。次が owner 担当なら、旧集合 A へ戻る場合も reopen します。
- `crates/ezsdr-radio-uhd/src/provider/core.rs:231–235`：prune は `stale` を消しません。中間 B がゼロ長で計画から省略されても、通知は残ります。
- 次が control-configured の成功 segment なら、`rx.rs:291–313` の既存経路で reopen せず集合を更新します。

したがって、**B の configure 失敗後に A へ戻る系列、成功した B が owner の読む前にゼロ長になる系列の両方に、修正は届いています**。

**分割テストも、この修正の検証として妥当です。** `control.rs:1095–1112` が印の生成・保存を、`rx.rs:1123–1139` が同集合でも reopen する消費側を固定しています。両者の間で、今回問題となる prune・segment 省略が印を失わせる経路はありません。ただし、操作列全体を実行する回帰テストではありません。

なお、四つの outcome の最後の `WrongRate` は **configure の rate read-back 不一致**であり、`check_rates()` の rate-set 照合失敗ではありません。報告の分類は不正確ですが、今回の閉鎖判断は変わりません。

P364–P367 は各 `old` が対象ファイルに一回一致することを確認しました。置換も対応する assertion を壊す内容です。kill の実行結果は再検証していません。

## 2. 指摘

1. **major — `configured` は「最後に device に適用された設定」を保証しない〔既存・延期可〕**
   - **場所：** `crates/ezsdr-radio-uhd/src/provider/control.rs:762–810,833`、`crates/ezsdr-radio-uhd/src/provider/core.rs:243–249`、`crates/ezsdr-radio-uhd/src/provider/rx.rs:291–314`
   - **根拠：** `configured` は segment ごとの過去の実施記録です。後続の from-none が同じ streamer を置換しても、古い segment のフラグは残ります。`stale` は owner の集合キャッシュを無効化しますが、この近道を無効化しません。
   - **系列：** owner を idle のまま止める → 将来の B=`{0,1}` を control が open/configure → さらに後の cold 更新で集合を空にし、B を正の長さで終わらせる → その後の C=`{0}` を control が from-none として open/configure → owner を再開する。最初に読む B は `configured=true` なので、実際には C の streamer・設定のまま B を開始します。**操作列の実行再現は未実施：INFERRED。**
   - **HEAD との比較：** `4199e51` にもあります。旧 `control.rs:557–581` は同じ from-none 判定とフラグ保存を持ち、旧 `rx.rs:283–289` も無条件に configure/open を省きます。旧キーの `radio.rx.channels` を `2 → 0 → 1` とする系列でも成立する構造です。
   - **修正：** segment の「一度設定した」ではなく、device に最後に成功適用した集合・設定との一致で省略を判断します。
   - **延期判断：** 1b-4d で同じ所有権境界を一般化するため、その冒頭で統合修正する方が合理的です。ただし、これを minor として却下・失念せず、上の系列を最初の回帰テストとし、修正前の状態を実機 parity の根拠にしないことが条件です。

## 3. 一記録へ統合する案

**方向は賛成です。重複するキャッシュと通知フラグを減らせます。ただし「完了後に共有記録を書くだけ」では不十分です。**

1. **device 操作の所有権を先に確保する必要があります。**  
   現在の control は idle を確認して line lock を解放します（`control.rs:793–801`）。その間に owner が古い有限 segment を開始できます。共有記録があっても、owner の一致確認後に control が device を置換すれば同じ不整合です。line lock 内で操作担当を確保し、完了まで他方が同じ stream の操作・開始に入らない仕組みが必要です。device 呼び出し中の line lock 保持は不要です。

2. **open 成功と configure 成功を分けて記録します。**  
   open 成功時点で集合を更新し、旧設定の一致記録を無効化します。全 channel の configure・read-back 成功後にだけ設定を記録します。途中失敗では設定を unknown にする必要があります。

3. **cold の `seq` だけでは設定の同一性になりません。**  
   `ColdConfig` は後から timed 更新の追加・移動・取消を受けます（`control.rs:94–100,633–638`）。同じ cold seq のまま投影が変わります。設定の版も比較するか、その変更時に一致記録を無効化してください。また、timed command の投入成功を「device が既に適用した」と記録してはいけません。

4. **削除対象はキャッシュであり、実行中 segment の情報ではありません。**  
   RX の block 解釈、TX の channel 数・ACK 対応に必要な情報は残ります。UR-13 の保存済み照合拒否も、新しい記録が省略してはいけません。

**推奨：1b-4c に統合改修を追加せず、1b-4d の最初の独立した手順として実施する。** 単に `stale` で `configured` を無視する追加パッチは、再 open を増やしつつ所有権問題を残すため勧めません。上記の所有権と設定同一性を先に設計メモ・テスト計画で固めれば、統合案は現状より単純になります。

未実施：テスト・mutation・実機の実行。判断は作業ツリーと HEAD の静的確認に基づきます。
