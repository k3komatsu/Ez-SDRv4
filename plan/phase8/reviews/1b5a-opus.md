# 1b-5a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

段階 1b-5（server・Python・fixture を layout の key へ移し，今の key のモードを消す）を 3 つに分けた（orchestrator，2026-10-21）．大きさは事前に数えた：今の key 専用の本体が約 450 行（`DeviceDescription`，Mock と UHD の `Mode::Current`，`channel_keys`），2 つのモードの分かれ目が `is_layout()` 34・`Mode::Current` の腕 44，今の key を使うテストが約 210 本，design の 6 つの spec に 76 行．

- **1b-5a（これ）**：server の既定の profile と Python の client．
- 1b-5b：MockRadio の今の key のモードを消し，Mock・acceptance・kernel のテストを移す．
- 1b-5c：UHD Module の今の key のモード，`DeviceDescription`，selector の `keys` を消し，UHD のテストを移す．

1b-5a でしたこと：

- **server**：既定の profile の radio の selector に `"keys": "channel"`（1b-5c で消す），capture Sink の feed と placement の link を port `rx0` に．
- **Python**（spec 16 EA-16，EA-17，plan 33 D11）：`Radio.rx_channels[n]`・`tx_channels[n]`（`frequency`，`gain`，`antenna`，`stream`），`Radio.rx_streams[k]`・`tx_streams[k]`（`sample_rate`，読むだけの `channels`，`start`・`stop`・`next_at`・`capture`・`request`・`result`，送信は `repeat`）．`Radio.rx`・`Radio.tx`・`Session.rx`・`Session.tx` は stream 0 の別名（owner 2026-10-21）．一覧と所属は Session の effective の key から作る（EA-18：binding の中身を読まない）．1 つの操作は 1 つの Action．`repeat` は波形の channel 数が stream の所属と違えば request の前に `Error`．宛先は `<radio>/<d>/s<k>`，recorder は port `rx<k>` から．
- **送信は明示して有効にする**：RM-30 の既定で送信の channel はどの stream にも入らない（今の「送信 0」と同じ）．今の `repeat` は `radio.tx.channels` を自分で変えていたが，layout では `tx_channels[0].stream = 0` を書く．Vision §3 の 2 つの例，§57，§66，`python/examples` に 1 行ずつ増えた．
- spec 16 と Vision（revision history に 1 行）．

実装は Luna（`openai/gpt-6-luna#max`），セルフレビューは DeepSeek（`opencode-go/deepseek-v4.1-flash#max`，owner 2026-10-21：OpenAI の 5 時間枠を節約する）．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 実装 | Luna，12:54〜14:30（セルフレビュー DeepSeek 2 回：P1 1 → なし） | `STATUS: blocked`（§3 の裁定 0） |
| Opus | 指示書への忠実さ，変えてはならない挙動，設計の意図 | P2 1，minor 3，process 1 |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**，major 1（＝Opus の P2） |
| 修正の回 1 | Luna，14:38〜14:55 | mutation 5/5 killed |
| Sol の再レビュー | 直したところと隣の系列 | major は解消．minor 1（Vision の 1 行）は orchestrator がその場で直した |
| gate | §4 | phase6 の P02・P03 が生き残った（§3 の最後） |
| 修正の回 2 | Luna，15:08〜15:36 | 5/5 killed |

## 3. 指摘と扱い

- **裁定 0（Luna の blocked）**：指示書 §2.1 の「既定の profile で送信 c0 が stream 0 に入っていること」は，§2.2 の「`repeat` は所属を変えない」と両立しない orchestrator の誤り．取り下げ，送信は明示して有効にする．既定で送信を有効にする案（RM-30 を変える）は，その既定が実機の UHD にも効くので退けた．Luna は Radio の crate を触らずに止まって報告した（指示書どおり）．
- **Sol 1（major）＝ Opus P2**：channel と stream の一覧を binding の selector から数え，stream の数を channel の数から決めていた（EA-18 に反する．selector が空の profile で一覧の長さが合わない，Sol が再現）．effective の `radio.<d>.c<n>.stream`・`radio.<d>.s<k>.sample_rate_hz` から作るように直した．
- **Opus minor**：Vision の例を別名のまま書く（足すのは送信を有効にする 1 行と channel の属性の書き換えだけ），削りすぎた docstring（`repeat` の PPS，`stop` の RM-16・RM-21）を戻す，エラー文から 1b-5c で消える `keys` を外す．すべて直した（Vision の 1 行は Sol の再レビューの minor として orchestrator が直した）．
- **gate：silence のまま通る照合（phase6 の P02・P03 の生き残り）**．EA-17 の 2 channel の往復のテストは，1b-5a の書き換えの後，受けた sample がすべて 0 だった：所属の変更（cold）で送信の stream の clock が T0＋50 ms・100 ms から始まるのに，5 ms 後に capture していた．照合の helper `rotation` は `y` が 0 だと位相の比が 0 になり，`assert_allclose(0, 0)` で一致としていた．tree の外の写しで helper に silence の検査を足すと，**HEAD でも 2 本**（`test_ea_17_requests_made_ahead_capture_contiguous_samples`，`test_v58_01_the_same_script_under_another_profile`）が silence で通っていた（Phase 6 からの検出力の穴）．修正の回 2 で helper が silence を不一致とし，3 本とも送信の clock の origin と経路の遅れを過ぎるまで Run の時間を進めてから受ける．
- **process（orchestrator）**：原因を調べるときに HEAD の server を build するため，main tree で `git stash`・`git stash pop` を使った．tree は戻ったが，戻ったファイルが mode 600 になり，Luna の remote mutation が 2 回 copy で失敗した（Luna が 644 に戻した）．HEAD の build は `git worktree` か `git archive` の写しでする．
- **process：Luna が orchestrator の指示書（`impl-brief.md`）を自分の要約で上書きした**．元の文は会話の記録から書き戻した（[1b5a-brief.md](1b5a-brief.md)）．修正の指示書から「orchestrator のファイルは読むだけ」を明記し，その回は守られた．次の段階の指示書にも入れる．

## 4. gate

テスト 1569，clippy，`rules.py` 0 件，Python 3.14・3.9 各 56（修正の回 2 の後にも流した），server 43．docker の uhd は省いた（UHD の crate は変えていない）．mutation の全リスト：§6．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| セルフレビュー（DeepSeek，2 回） | P1 1 | P2 4，P3 多数 | テストの観測（送受信の所属の対称）．機構なし |
| Opus | 0（P2 1） | 3 | 指示書の誤り（裁定 0）．client が binding の中身を読む（下）．機構なし |
| Sol | 1 | 1 | client が binding の中身を読む．`mech:fact-stated-twice-in-spec` に近いが，spec の文の重複ではなく client が Provider の知識を持つ形．数えない |
| gate（mutation） | 0 | 1 | silence を一致とする照合の helper（HEAD からの 2 本を含む）．テストの検出力．機構なし |

`failure-mechanisms.md` に足すものはない．

## 6. mutation の全リスト

全リスト（修正の回 2 の前）：maintenance 902/902，phase3〜5 全部，phase6 61/63（P02・P03．§3），phase7 164/165（G09，既知）．修正の回 2 の後，P02・P03 と同じ ID の maintenance の行，J186 を流し直して 5/5 killed．

## 7. 観察

- DeepSeek のセルフレビューは 2 回で終わり，OpenAI の 5 時間枠は実装と修正で 18% ほどだった（Luna が自分でレビューした 1b-4e-2 は枠を 2 回使い切った）．
- Luna は指示書の要求が満たせないとき，範囲の外を触らずに `blocked` で止まった．
