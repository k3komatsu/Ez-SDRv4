# 1b-4d-2：orchestrator のレビュー，裁定，数

## 1. 何をしたか

layout の stream 1 以上を stream 0 と同じに動かした（plan 33 の「1b-4c で 1b-4d へ送るもの」(1)〜(3)，(5)，(7)，(12)）．

- `prepare` は宣言した全 stream を回る：link は port `rx<k>` ごと（`Core::links(k)`），SampleClock は `<id>/rx/s<k>`・`<id>/tx/s<k>`，RX_OVERFLOW の handle も stream ごと．
- `arm` は各 stream の streamer を開き，`start` は各受信 stream の timed start を出して，宣言した stream ごとに `uhd-rx-s<k>`・`uhd-tx-s<k>` を立てる（今の key は `-s0` だけ）．途中で失敗したら，loss を公開してから，始まった owner を `stop(Abort)` で止める．
- `stop` は uhd-control → TX を `k` の順 → RX を `k` の順に join する．1 本でも detach したら，device 全体の `close_streams` は呼ばない．
- uhd-control は `Stop`・`start_rx`・burst を宛先の stream に記帳する．timed 更新は，その instant に channel を持つ stream の line で判断する（`channel_live`，全 stream の `targets`）．queue の depth は instance で数える（UR-24 の近似のまま）．
- stream から stream への移動は `UR-25:` で拒否する（1b-4e まで）．`channel_in_later_stream` と `[PreparedStream; 1]` の guard は消した．
- owner は configure から戻った後，timed start（RX）と clock の採用（TX）の前に loss を見る．

## 2. 経過

| 回 | 中身 | 結果 |
|---|---|---|
| 計画 | DeepSeek，26 分．指示書の誤り 6 点を根拠つきで直した（LATE_COMMAND は RM-11 で device node，S3 の timed start は owner，S12 は End の床など），系列を 4 本足した | Sol：要修正，major 7・追加 major 2・minor 5 |
| 裁定 | stop の再確認（指示書 §3.7）を取り下げ，今の挙動を固定する．他の major は採る．計画の再レビューはせず，v2 をコードと一緒に見る | — |
| 実装 | DeepSeek，3 時間 2 分（計画 v2，セルフレビュー 3 回を含む） | Opus：指摘なし．Sol：FAIL，major 2・minor 2 |
| 修正 1 | start の途中の loss，S16 の観測，gate の解放，UR-23 | Sol：**PASS** |

## 3. 裁定の要点：stop と owner の窓

1b-4d-1c から送られた窓（owner が `Planned` を取った後に `Provider::stop` が来る）に，指示書は「owner が configure の後に stop を見直して始めない」を置いた．Sol の計画レビュー（major 1）が，これは窓を狭めるが閉じないことを示した．

取り下げた理由：残る状態（cut された segment に timed start が queue されている）は，**owner が origin より前に begin した segment に `Provider::stop` が来れば，窓がなくても起きる**．この状態の扱いは spec 22 VH-6（#56）が決めている（uhd-rx は origin の後で untimed に止め，届いたものを捨てる）．見直しは保証を変えずに仕組みを 1 つ足すだけになる．S16 は今の挙動を固定した（RX は unit で，owner が detach されずに終わること；TX は終わった clock の burst が送られないこと）．

**未修正の穴（[#67](https://github.com/k3komatsu/Ez-SDRv4/issues/67)）**：origin が遠い（join の 1 s より先）segment の timed start が queue されたまま `Provider::stop` が来ると，uhd-rx は origin まで待つので join の 1 s を越えて detach され，cleanup は `close_streams` を呼ばない（streamer が漏れる）．HEAD からある挙動で，この段階の窓に限らない．直し方は #56 の bench（queue された timed start を持つ streamer を閉じたとき X310 が何をするか）を待つ．

## 4. 数（AGENTS.md §9 の 8）

| 出どころ | major | minor | 機構 |
|---|---|---|---|
| Sol の計画レビュー | 9（7＋追加 2） | 5 | stream-lifecycle-scattered 1（major 1：遠い origin の detach，§3 の穴．未修正），残りはテストの観測と design の文 |
| セルフレビュー（DeepSeek，実装 3 回・修正 1 回） | P1 1 | P2 8，P3 多数 | P1 は S12 の `delivered` の観測 |
| Opus | 0 | 0 | — |
| Sol のコードレビューと再レビュー（2 回） | 2 | 2 | stream-lifecycle-scattered 1（start の途中の loss を公開しない），テストの観測 1（S16） |

`stream-lifecycle-scattered` は 3 件を超えて設計メモ（spec 22）があるので，この段階では数を足すだけにする．loss の検査が経路ごとに散っている（`Core::configure`，release，owner の configure の後，start）のは，この機構の形そのものである．1b-5 の後の見直しの候補．

計画を先に書く手順の評価：計画の段階で major 9 件をコードの前に潰した．コードの後の major は 2 件で，うち 1 件（S16）は計画の段階の指摘を実装が満たしきれなかったもの．1b-4d-1c（コードの後に major 6 件）より往復が短い．計画の再レビューを省いたことによる手戻りは見えない．

## 5. 持ち越し

- **遠い origin の detach**（§3）：#67．#56 の bench の系列に足す．
- spawn 自体の失敗（thread を作れない）は決定的に作れず，テストが無い．
- `with_rx_stall` は stream 0 だけ（B5 用，design に明記）．
- `S16a` の e2e は detach を許す形のまま残した（統合の経路を見るテスト）．joined の経路は unit が固定する．
- 1b-4d-3 へ：event と Manifest の plan の stream 番号（plan 33 の (4)，(8)，(9)）．
- bench へ：H1（X3xx で 2 本の rx streamer），queue が RFNoC block ごとであること（H3，UR-24 の近似）．

## 6. DeepSeek V4.1 Flash（2 段階目の観察）

- 計画 26 分，実装 3 時間 2 分（セルフレビュー 3 回を含む），修正 43 分．
- 指示書の誤りを根拠つきで直した（RM-11）．裁定に無い仕組みは足さなかった（指示書に「足さない」を書いた）．
- mutation の初回の生き残り 6 行（P454・P455・P458・P460・P464・P472）は自分で直した．修正の回は 0．
- 報告は正直だった：修正の回 1 で，前の TX のテストが plan の空の状態で何も守っていなかったことを自分で書いた．
- 計画外の変更が 1 件：`rules.py` が指摘した前の段階の記録の壊れたリンク（`1b4d1c-test-plan.md:384`）を直した．報告に書いてあり，害はない．
- セルフレビューは P1 が無くなった後も 3 回目まで回した（指示書の上限内）．
- 修正の回で流した行の外で，既存の 2 行（P324，P342）がコードの形の変化で当て先を失っていた（gate の全リストで分かった．orchestrator が当て直した）．触ったコードを引く既存の行の洗い出しが足りなかった．
