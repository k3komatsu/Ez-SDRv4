# 1b-6a：orchestrator のレビュー，裁定，数

## 1. 何をしたか

設計メモ [35](../35-uhd-stream-state.md) の R1．UHD Module の stream ごとの印 `busy`・`busy_seq`・`reading` を，private な `Claims { holder, reading }`（`crates/ezsdr-radio-uhd/src/provider/core.rs`）にし，`Streams` の遷移の関数（`claim_owner`，`claim_release`，`claim_from_none`，`may_call_from_none`，`begin_read`，`begin_shutdown_read`，`end_read`，`give_back`）だけが変える．

- `Holder { Free, Owner { seq }, Release, FromNone }`．uhd-tx の report の読み（`reading`）は holder と別に重なりうる（読みの最中の from-none の待ちと中断，`shutdown` の読み）．
- 取る関数は `Claim`（`#[must_use]`）を返し，`give_back` だけが消費する．`owner_configure` は失敗で `Err((error, claim))` を返し，呼び出し側（`Rx::begin`，`Tx::do_switch`）が同じ lock の節で拒否を book してから返す．`Claim` の Drop は test build でだけ返し忘れを panic で知らせ（他の panic の巻き戻し中は除く），production では何もしない．lock を取り直さない．
- production の `busy = false` 9 か所と `reading` の 4 か所はすべて関数の呼び出しになった．`Tx::read_reports_before_close` は `begin_read` が取れないとき読みを飛ばす（計画の決定 F．holder が `Free` でない時に close 前の読みに来る道は今は無い）．
- spec 18 UR-25 の Ownership の段落，`*Test:*` 行，Changes．
- テスト：新しく 14 本（`Streams` の遷移の unit test 10，拒否の節での holder の観測 2，`shutdown` の読みと戻ってくる uhd-control の両順序 2）．既存のテストで印を直接読み書きしていた所は関数と観測に置き換えた（削除・改名なし）．probe を 3 つ足した（`reading_wait`，`refused_booking`，`release_claimed`．test build でだけ働く）．
- mutation の行 22 行を当て直した（計画の 14 行と，書き換えで `old` が一致しなくなった 8 行）．新しい行は無い．

本体とテストで約 +720／−200．実装は DeepSeek V4.1 Flash（`opencode/deepseek-v4.1-flash-free#max`，owner 2026-10-22），セルフレビューも同じモデル（1 回，P1 なし）．free の run は常に 1 本で，上限には当たらなかった．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 設計メモ | Opus | Sol のレビュー 2 回（修正して採用：major 4＋2，minor 2＋1．すべて採用，メモ §7） |
| テスト計画 | DeepSeek，12:11〜12:34 | 系列 S1〜S8 に既存テストと新しいテスト 12 本，未決 A〜G |
| Sol の計画レビュー | 自分の失敗の筋書きから | **FAIL**，major 3・minor 3 |
| 計画の修正と実装 | DeepSeek，12:38〜13:44 | セルフレビュー 1 回（P1 なし，P2 1・P3 1 を直した），当て直した 22 行 22/22 killed，新しいテストを 20 回続けて流して 20/20 |
| Opus | 指示書への忠実さ，変えてはならない挙動 | major なし，minor 1 |
| Sol のコードレビュー | 自分の失敗の筋書きから | **FAIL**（major なし），minor 1 |
| 修正 | Opus が直した（minor 2 つ，文とコメントと Drop の 1 行） | |
| gate | §4 | 1260/1261（G09 は既知） |

## 3. 指摘と扱い

計画レビュー（Sol，すべて採用）：
- **major 1**：S4a は `wait_reading` probe（待ちの loop の前）で観測していて，loop の中で `reading` を見ない誤実装でも通った．loop の中の判定の直後に `reading_wait` probe を置き，その時点で open していないことを確かめてから読みを解放する．
- **major 2**：拒否の book の前に返す誤りは，lock の回数と最終状態では見えない．RX・TX の両方で，拒否の節に入った時に stream がまだ `Owner { seq }` を持つことを `refused_booking` probe で観測する．
- **major 3**：`release_unneeded` で取った後に loss が来る道を，計画は「決定的に踏めないので review だけ」としていた．`release_claimed` probe で loss を publish して再開するテストを足した．
- minor 3：遷移の表の「前が違うとき」の抜け，N8 の二重 panic，N9 の準備（受信 state が 1 つ，line が無い）．
- 未決 A〜G は Sol の推奨どおりに決めた．

実装の後：
- **O1（minor）**：test build の `Claim` の Drop が，他の panic の巻き戻し中にも panic し，test binary が abort して落ちたテストの名前が分からなくなる．`std::thread::panicking()` の間は panic しないようにした．
- **Sol（minor）**：spec 18 とコメントが，`Release` を取るのは uhd-control だと書いていた．`release_unneeded` を呼ぶのは idle の stream の owner（`Rx::begin`，`Tx::pass`）．文を直した．
- **process**：実装役は計画の修正節を実装の後に書いた（指示は「直してからコードを書く」）．中身は Sol の指摘どおりで，コードとテストはそれに従っている．free の DeepSeek は 1 回の run で計画の修正から実装・セルフレビュー・mutation まで済ませた．

## 4. gate

テスト 1570，clippy，`rules.py` 0 件，docker の uhd（clippy，lib 259，`uhd_api` 5），Python 3.14・3.9 各 56，mutation の全リスト 1260/1261（G09 だけが既知の survivor）．この段階で壊れた行は無かった．

## 5. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol（設計メモ，2 回） | 6 | 3 | 設計の表し方（読みと holder の重なり，`needed_before`，返す順序）．機構は `stream-lifecycle-scattered` への答えの形そのもの |
| Sol（テスト計画） | 3 | 3 | テストの観測の穴．機構なし |
| セルフレビュー（DeepSeek，1 回） | P1 0 | P2 1・P3 1 | テストの観測．機構なし |
| Opus | 0 | 1 | test build の Drop．機構なし |
| Sol（コード） | 0 | 1 | 文の誤り．機構なし |
| gate（mutation） | 0 | — | — |

計画を先に書く手順（AGENTS.md §9 の 2・3）は，この段階では計画レビューで major 3 を拾い，コードレビューでは major 0 だった．
