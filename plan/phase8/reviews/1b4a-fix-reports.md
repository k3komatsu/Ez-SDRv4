# 1b-4a の修正の報告

## 修正の回 1

STATUS: done

# Phase 8 段階 1b-4a 修正の回 1 の報告

指示書: `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/fix-brief.md`（HEAD `9816d78`，未コミットの作業ツリー）．裁定で採られた 7 件（Sol 1〜6，Opus O1〜O3）をすべて直した．

## 1. 変えたファイルと要点

| ファイル | 要点 |
|---|---|
| `crates/ezsdr-radio-uhd/src/uhd.rs` | **修正 1**：`made_subdev_spec` を足し，`uhd_subdev_spec_make` が失敗したときも，UHD が解析の前に確保した handle（`subdev_spec_c.cpp:31-36`）を非 null なら解放してから error を返す．`set_subdev_spec` はこの helper を使う．feature `uhd` のテスト用に `subdev_spec_lifecycle`（device なしで make/free の経路を走らせる）を足した（`lib.rs` で公開）． |
| `crates/ezsdr-radio-uhd/src/device.rs` | **修正 2**：fake の `rx_open`・`tx_open` は，検査の前に空の列を `[0]` に読み替える（`UhdDevice` と UHD の `sanitize_stream_args` と同じ）．**修正 3**：timed command の宛先を `(slot, front end)` にし（`commands` の型），`apply` は booking 時の spec の front end を，`settle` はその front end が今の spec の何 channel かを引いて適用する（外れていれば何もしない）．**修正 4**：`parse_spec` の区切りを ASCII の空白だけにし（空 token は飛ばす），3 分割以上の失敗の呼び出し名を `uhd_subdev_spec_make` にした（表にない組は `uhd_usrp_set_*_subdev_spec` のまま）． |
| `crates/ezsdr-radio-uhd/tests/fake.rs` | **修正 5**：`ur_33_two_receive_streamers_keep_their_own_channels` を強めた（stop の後に stream 0 が stop の時刻より十分先まで届けること，stream 1 の tail の後に Timeout になること．どちらも負荷に関係なく P216 を kill する）．`ur_33_setting_a_spec_disconnects_that_direction_s_streamers` に送信側を足した．**修正 2・3・4 のテスト**：`ur_33_an_empty_channel_list_is_channel_zero`，`ur_33_a_timed_command_follows_its_front_end_across_a_spec` を足し，`"A:0\tB:0"` の拒否と `uhd_subdev_spec_make` の文面を `ur_33_a_subdev_spec_uhd_would_refuse_is_refused` に足した． |
| `crates/ezsdr-radio-uhd/tests/uhd_api.rs` | 修正 1 のテスト `uhd_api_a_refused_subdev_spec_is_freed_once`（拒否された markup の文面と呼び出し名，make/free の経路．free 自体は in-process では見えない）． |
| `design/18-uhd-radio.md` | **修正 6**：UR-3 に「`set_subdev_spec` は make の成否にかかわらず handle を解放する」と「streamer のない stream への呼び出しは `UhdDevice` では即時に失敗し，`tx_async` は即座に report なし」を足し，Test 行に `uhd_api_a_refused_subdev_spec_is_freed_once` を足した．UR-15・UR-17・UR-25 の呼び出しを `rx_start(0, T0)`・`rx_recv(0, …)`・`rx_packet_samples(0)` に直した．UR-33 の規範の文から理由・段階の話・「(the M-rules below need only the first)」を外し，規範を「fake が何をするか」＋短い出典にした（空の列は channel 0，timed command の宛先は front end）．§6 の表の該当行を直し，2 つの新テストの行を足した．Changes 行を直した（`UhdDevice` は stream の番号ごとに 1 つ／UR-15・17・25 は stream 0／外した理由の記録／文言の訂正）． |
| `plan/phase8/33-channels-and-streams.md` | §5 の 1b-4 行に 1b-4a の記録を足し，規範から外した理由（fake の拒否が UHD より厳しい出典，残った front end の設定が保たれる理由，`serial` の channel 0，待ちの理由）を記録した．空の列について HEAD の `UhdDevice` が長さ 0 の配列を渡していたことも記録（未定義動作とは書かない）． |
| `plan/maintenance/tools/mutations.json` | 新規 P230〜P235（下表）．P209 を当て直し（`old` が `uhd_subdev_spec_make` の文面に変わった）．P232 は 2 編集（`old`/`old2`）． |

## 2. 直したこと（fix-brief の番号ごと）

1. **handle の漏れ**：`made_subdev_spec`（`uhd.rs`）．make の成否にかかわらず，非 null の handle を 1 回解放する．テスト：`tests/uhd_api.rs` の `uhd_api_a_refused_subdev_spec_is_freed_once`（拒否された markup の error が `uhd_subdev_spec_make:` で始まり `invalid subdev-spec markup string` を含むこと，受け入れた markup が作られて解放されること．free そのものは in-process の witness がないので，設計の Test 行にその限界を書いた）．mutation の行：feature `uhd` のコードには作れない（gate は default feature の `--test fake`）．
2. **空の channel 列（fake）**：`rx_open`・`tx_open` が検査の前に `[0]` に読み替える．二重の open と範囲の検査は channel 0 として行う．テスト：`ur_33_an_empty_channel_list_is_channel_zero`（`rx_open(0, &[])` の後の `rx_recv` が vector 1 本／`tx_open(0, &[])` の後の buffer 1 つの `tx_send` が受理／`rx_open(0, &[0])` の後の `rx_open(1, &[])` と，送信の同じ形が拒否）．mutation：P230（受信），P231（送信）．UR-33 の該当の文と Changes 行を「空の列は UHD と同じく channel 0」の 1 つの規則に直した．
3. **timed command の宛先**：予約の宛先を `(slot, front end)` に持ち，効く時にその front end の今の channel を引いて適用し，spec から外れていれば何もしない（`spec に戻ったら既定値`の規則は変えない）．テスト：`ur_33_a_timed_command_follows_its_front_end_across_a_spec`（`"A:0"` に縮めた場合は panic せず channel 0 も変わらない／`"B:0 A:0"` の並べ替えでは予約した front end の channel に効く）．mutation：P232（booking が channel の番号を持ち，settle がその番号で引く形；2 編集）．1 回目の self-review の指摘（名前と中身のずれ）で，名前どおりの mutant に直した．
4. **markup の区切りと呼び出し名**：`split_whitespace()` をやめ `split(' ').filter(!is_empty)` に（UHD の `boost::char_separator<char>(" ")`，`subdev_spec.cpp:19-20`）．3 分割以上の失敗の呼び出し名は `uhd_subdev_spec_make`，表にない組は `uhd_usrp_set_*_subdev_spec` のまま．テスト：`"A:0\tB:0"` の拒否を `ur_33_a_subdev_spec_uhd_would_refuse_is_refused` に足した．mutation：P234（空白分割への後退），P235（失敗の呼び出し名）．
5. **テストの力**：`ur_33_two_receive_streamers_keep_their_own_channels` を manual clock のまま強め，stop の後「stream 0 が stop の時刻より十分先（stop+1 ms）まで届け続ける」「stream 1 は tail の後 Timeout になる（止めた後に生成された sample を届けない）」を確かめる．どちらも P216（rx_stop が stream 0 を止める）を負荷に関係なく kill する（mutant の stream 0 は stop の時刻以降の sample を決して届けられず，mutant の stream 1 は止まらないので Timeout が来ない）．送信側：`ur_33_setting_a_spec_disconnects_that_direction_s_streamers` に `set_subdev_spec(Tx, …)` で送信の streamer が切れ，受信が残ることを足した．mutation：P233．
6. **spec 18 の文（DA-1）**：UR-33 の今回の文から理由・「this stage」・意味の取れない句を外し，Changes 行と plan 33 §5 に移した（他の理由＝front end の設定が保たれる理由，`serial` の channel 0，fake が待つ理由，空 spec・重複の拒否が出典つきで UHD より厳しいことも記録）．UR-15・UR-17・UR-25 の呼び出しを新しい署名（stream 0）に直した．`design/` を grep して他の古い署名がないことを確かめた．Changes 行の「holds one streamer per direction」を「one streamer per stream number」に直した．
7. **報告に足すこと**：§3-3 に書いた（`rx_packet_samples` の 0，`TxReport` fault の条件，ほかに 0 channel の引数と loopback）．

## 3. 指示書から外れたところと理由

1. **`fix-brief` 4 のテストに加えて mutation の行を 2 つ足した**（P234 空白分割，P235 失敗の呼び出し名）：self-review のチェックリスト「規則ごとにテストと mutation の行」の指摘で，item 4 が入れた規則を gate で守るため．
2. **`fix-brief` 6 の DA-1 の掃除を，指示の 3 点より広く当てた**：self-review が UR-3 の handle の文・`serial` の理由・UR-33 の channels/transmit の理由を挙げたので，すべて規範の外（Changes 行・plan 33）に移した．ほか：UR-3 の last-error の文を「usrp か streamer の handle を持たない呼び出し（meta-range と subdev spec を含む）」に直した（P3 の指摘：`rates` の meta-range 呼び出しは `check` で読むため）．
3. **fake の振る舞いの変更で報告に足したもの**（fix-brief 7 の 2 つに加えて）：
   - `rx_packet_samples` は，streamer がないとき 0 を返す（`UhdDevice` と同じ）．
   - 予定された `TxReport` の fault は，送信の streamer が開いているときだけ出る．
   - `UhdDevice`（と fake）は，0 channel への切り替えの `tx_open(0, &[])`／`rx_open(0, &[])` で channel 0 を C に渡す（HEAD は長さ 0 の配列を渡していた．UHD は `sanitize_stream_args` でどちらも `{0}` と読むので機能は変わらない）．値は段階の report §3-5 にある．
   - fake の loopback が channel ごとになったこと（受信 `c` ← 送信 `c`）は段階の report §1 のとおり（この回では変えていない）．
4. **当て直した行**：P209 のみ（`old` を `{call}:` から `uhd_subdev_spec_make:` の文面に．壊す規則は同じ「3 分割以上を受け付ける」）．P232 は新規行（2 編集版）．段階で当て直した J323・J326・J327・B01・B15・B20・B22・B28・B30・R12 はこの回で触っていない（self-review が 1 回一致を確認）．
5. **valgrind は image にない**（`docker run … which valgrind` で確認．ASan は nightly が要る）．修正 1 の free は in-process の witness がないため，テストは経路と文面だけを pin している（設計の Test 行に明記）．

## 4. 規則ごとのテストと mutation の行（この回に足したもの）

| 規則（spec 18） | テスト | mutation の行（誤りの形 → kill するテスト） |
|---|---|---|
| 空の列は channel 0（UR-33） | `ur_33_an_empty_channel_list_is_channel_zero` | P230 は受信・P231 は送信で，空の列を空のまま開く → 同テスト |
| timed command の宛先は front end（UR-33） | `ur_33_a_timed_command_follows_its_front_end_across_a_spec` | P232 booking が channel の番号を持ち settle がその番号で引く（2 編集）→ 同テスト（縮小の場面は範囲外 panic，並べ替えの場面は別 channel に効く） |
| spec 変更はその方向の streamer を切る（UR-33） | `ur_33_setting_a_spec_disconnects_that_direction_s_streamers`（送信側を追加） | P233 送信側を切らない → 同テスト |
| markup は ASCII の空白だけで区切る（UR-33） | `ur_33_a_subdev_spec_uhd_would_refuse_is_refused`（`"A:0\tB:0"`） | P234 `split_whitespace()` に戻す → 同テスト |
| 不正な markup の失敗の呼び出し名は `uhd_subdev_spec_make`（UR-3，UR-33） | 同上（`"A:0:1"`） | P235 `{call}` に戻す → 同テスト |
| 空の列は channel 0・timed command の宛先（P216 の停止の範囲） | `ur_33_two_receive_streamers_keep_their_own_channels`（tail と stop の後を追加） | P216 は既存（この回に当て直しなし）．新しい検査で負荷に関係なく kill される |

`old` がちょうど 1 回一致することは全 6 リスト（994 行，`old2` を含む）で機械的に確認した（`rules.py --base HEAD` も 0 problem）．

## 5. 確かめられず INFERRED にしたこと

段階のときの INFERRED（pre-chain の rate，重複・空 spec の拒否，同じ channel の二重 open，`serial`，Basic/LF の名前，front end の設定の保持，`UhdDevice` の新しい経路）は変わらない．この回で足したもの：
- 空の列を channel 0 と読むのは UHD の `sanitize_stream_args`（ソースで確認）で，実機の挙動は未確認．
- timed command が radio block（front end）のものだという点（fix-brief の理由）はソースの読みに基づく INFERRED．実機は BP0．
- `uhd_api_a_refused_subdev_spec_is_freed_once` は double free も leak も観測しない（in-process の witness なし．valgrind は image になく ASan は nightly）．

## 6. 確かめること 1〜6 の結果（最終ツリー，すべて最後の編集の後に同じツリーで実行）

| # | コマンド | 結果 | ログ |
|---|---|---|---|
| 1 | `cargo test --workspace --no-fail-fast` | 65 suite すべて ok，1275 passed / 0 failed（新テスト 2 本で +2） | `check1-fix.log` |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | warning/error 0 | `check2-fix.log` |
| 3 | docker（`ezsdr-v4-dev:uhd4.10`）: clippy `--features uhd --all-targets` → `--test uhd_api` → `--lib` | clippy 0，uhd_api 5 passed / 0 failed，lib 106 passed / 0 failed | `check3-fix.log` |
| 4 | `cargo build -p ezsdr-server` → `PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=… ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests` | 47 tests，OK | `check4-fix.log` |
| 5 | `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` | 0 problem | `check5-fix.log` |
| 6 | `REMOTE_SSH="ssh -o BatchMode=yes" python3 …/remote_mutate.py maintenance sim02:8,sim03:8 <変えた・足した 25 行>` → 最後に `all sim02:8,sim03:8` | 25/25 killed；`all` は 993/994 killed（maintenance 618/618，phase3 83/83，phase4 34/34，phase5 29/29，phase6 63/63，phase7 166/167）．生き残りは既知の `phase7/G09` のみ（この回で触っていない） | `check6-rows-fix.log`，`check6-all-fix.log` |

## 7. セルフレビュー（AGENTS.md §9 手順 2，Haiku 5.5 `anthropic/claude-haiku-5-5#high`，read-only，3 回）

- **1 回目**（`fix-selfreview-1.log`）：P0 なし，P1 1 件（DA-1 が未完：UR-3・UR-33 の規範の文に理由が残る），P2 5 件（空白分割と失敗の呼び出し名の mutation の行がない／`rx_packet_samples` の test・行がない／streamer のない呼び出しの待ちの test・行がない／`UhdDevice` は即時に失敗するのに UR-33 が「UHD と同じ」と書く／leak の実行検証なし／「one per direction may be open per stream number」の読みにくさ）．直した：DA-1 の掃除（理由は Changes 行と plan 33 へ），P234・P235，UR-3 の「即時に失敗する」文，文言．残した：`rx_packet_samples`・待ちの test/行，leak の実行検証（§9 に未解決として書く）．
- **2 回目**（`fix-selfreview-2.log`）：P0 なし，P1 2 件（`rules.py` が `design/18:284` の 1 502 字で失敗／検証がまだ記録されていない），P2 5 件（0 channel の変化が報告にない／antenna 読み戻しが新しい失敗経路／fake の loopback が一覧にない／P232 の名前と中身のずれ／`uhd_api` の説明が leak も二重解放も見ていない／`rx_recv` の添字 panic の可能性）．直した：Test 行を縮めた（`rules.py` 0 problem），P232 を 2 編集の名前どおりの形に，`uhd_api` の説明，plan 33 と Changes 行への 0 channel の記録．残した（未解決として報告）：`rx_packet_samples` の test/行，待ちの test/行，valgrind，antenna の失敗経路（D16 の読み戻しで，記録は 1b-4c），`rx_recv` の panic（trait の前提が `close_streams` と `set_subdev_spec` に明記されている）．
- **3 回目**（`fix-selfreview-3.log`）：P0 なし，P1 1 件（最後の編集の後の検証がまだ＝この §6 の実行），P2 2 件（UR-33 が待ちを UHD の挙動として書いている／plan 33 の「未定義動作になりうる」は根拠が弱い），P3 数件．直した：UR-33 の待ちを fake の挙動として書き分け（`UhdDevice` は UR-3 のとおり即時），plan 33 を `sanitize_stream_args` の根拠に直した．P3 の扱いは §9．

## 8. 未解決のこと（1b-4b・1b-4c・owner へ）

- **自己レビューの残り**：`rx_packet_samples` が streamer のないとき 0 を返す規則のテストと mutation の行；streamer のない呼び出しの待ち（`rx_recv` の 10 ms，`tx_async` の timeout）のテストと行；修正 1 の free の実行検証（valgrind は image になく，ASan は nightly；feature `uhd` のコードは mutation gate の外）．
- **1b-4c**：untimed の antenna 読み戻し（D16）は今のところ誰も読まない（記録は 1b-4c）．読み戻しの失敗で `apply` が失敗する経路の扱いもそこで決める．`set_subdev_spec` は spec を streamer より先に設定する前提（この段階の fake と `UhdDevice` は spec の設定でその方向の streamer を切る）．`opened`（`rates` の pre-chain の履歴）は spec 変更で消える規則もそこで確認する．
- **1b-4b**：段階の report §9 と同じ（`UhdRadio` の呼び出しは stream 0，`Scripted` は stream 番号を無視，channel 数を 0 にする cold change は `tx_open(0, &[])` を呼ぶ）．
- **P3（採用しなかった self-review の任意の指摘）**：fake の `close_streams` は `opened` を消さない（UHD で未確認，触らない）；UR-33 の段落が長い（既存の書き方）；`serial` は送信だけの装置では失敗しうる（INFERRED と明記済み）．
- **BP0（bench）**：段階の report §9 と同じ（pre-chain の rate，reconnect の文面，`serial`，front end 名，antenna の読み返し）．

## 修正の回 2

STATUS: done

# Phase 8 段階 1b-4a 修正の回 2 の報告

指示書: `~/.cache/ezsdr-fixes/2026-10-18-phase8-1b4a/fix2-brief.md`（HEAD `9816d78`，未 commit の作業ツリー）．裁定で採られた Sol の再レビューの 3 件をすべて直した．あわせて，セルフレビュー 3 回の指摘（P1 は計画の文と証跡，P2 は引用・文言・mutation の行の追加）を直した．

## 1. 変えたファイルと要点

| ファイル | 要点 |
|---|---|
| `crates/ezsdr-radio-uhd/src/device.rs` | **修正 1**：`set_subdev_spec` が，成功して spec を置き換える直前に，古い spec のまま `self.settle(&mut st);` する（`parse_spec` の後，`std::mem::replace` の前．失敗した spec は何も変えない）．**修正 3**：unit test を 4 本足した（`ur_33_a_command_due_while_its_front_end_was_out_never_applies`，`ur_33_a_recv_on_a_stream_with_no_streamer_waits_its_10_ms`，`ur_33_a_report_read_on_a_stream_with_no_streamer_waits_its_timeout`，`ur_33_a_packet_size_read_of_a_stream_with_no_streamer_is_zero`．すべて manual clock の participant の上）．self-review の指摘で引用（`db_basic_and_lf.cpp:101-111`）と文言（「a refused reopen leaves the stream with none」）も直した． |
| `crates/ezsdr-radio-uhd/src/uhd.rs` | **修正 2**：`ffi_tests` に `uhd_subdev_spec_make`・`uhd_subdev_spec_free`・`uhd_usrp_set_{rx,tx}_subdev_spec`・`uhd_usrp_last_error` の置き換えを足し（この lib test binary の中だけ），成功・解析の失敗・set の失敗の 3 経路で make 1 回・free 1 回を数える `uhd_a_subdev_spec_is_freed_once_on_every_path` を足した．`subdev_spec_lifecycle` と `streamer_lifecycle` のコメントを，UHD の free が NULL を入れる事実に合わせた（`subdev_spec_c.cpp:41`，`usrp_c.cpp:97-101`）． |
| `crates/ezsdr-radio-uhd/tests/uhd_api.rs` | 2 つの lifecycle テストのコメントを，実際に見ているもの（実 libuhd の文面と経路が走ること．free の回数は ffi テストが見る）に直した．テスト名と本体は変えない（名前を変えると `rules.py --base HEAD` の引用の検査が落ちる：base が指す名前が head のテストでなくなるため）． |
| `crates/ezsdr-radio-uhd/tests/fake.rs` | 引用の訂正（`subdev_spec.cpp:19-20,58-59`）と文言（「a refused reopen leaves the stream with none」）だけ．テストの assertion は変えない． |
| `design/18-uhd-radio.md` | UR-33 に 2 つの規範の文：spec の変更は期限の来た予約を離れる側の spec で settle する（INFERRED），streamer のない呼び出しの答えに `rx_packet_samples` の 0 と fault の限定．UR-3・UR-33 の `*Test:*` 行，§6 の行（`ur_33_a_command_due_...` と `uhd_a_subdev_spec_is_freed_once_on_every_path` を追加，`uhd_api` の行と `src/device.rs` の段落を訂正），Changes 行に fix round 2 を追記． |
| `plan/phase8/33-channels-and-streams.md` | §5 の 1b-4 行に修正の回 2 の記録（settle の規則と INFERRED，境界のテスト，free の数え方），「`UhdRadio` の挙動は変えない」に 0 channel の例外を追記，引用の訂正． |
| `plan/maintenance/tools/mutations.json` | P236（`set_subdev_spec` の settle を外す），P237（streamer のない stream に packet 数を返す），P238・P239（待たずに返す），P240（`rx_recv` が上限なしで待つ），P241（`tx_async` が 10 ms だけ待つ）． |

## 2. trait の新しい署名

この回は変えていない（修正の回 1 の署名のまま．`cargo test --workspace`／clippy／docker の feature `uhd` が通ることで，新旧の呼び出しが揃っていることを確かめた）．

## 3. 指示書から外れたところと理由

1. **fix2-brief 2 で，make/free に加えて `uhd_usrp_set_{rx,tx}_subdev_spec` と `uhd_usrp_last_error` も置き換えた**：set の失敗の経路は，dummy の usrp handle が null のまま「失敗する set」を作る必要がある．実物の set は `USRP(h)->set_...` で h を deref し（`usrp_c.cpp:563-568`），失敗時に読む `uhd_usrp_last_error` も `h->last_error` を読む（`:250-254`）ので，置き換えないと segfault する．置き換えは `#[cfg(test)] mod ffi_tests` の中だけに効く（`tests/uhd_api.rs` は実物をリンクする．docker の `--test uhd_api` が 5 passed で確認）．
2. **修正 1 のテストは `tests/fake.rs` でなく `src/device.rs` の unit test に置いた**：booking と 2 回の spec 変更の間で host の負荷が clock を動かさないよう，全部を 1 つの manual clock の participant で走らせるため（`tests/fake.rs` の integration test は `HostClock::spawn` を呼べない）．mutation の行の `test_bin` は `lib`．両方の場合（T より後に戻す／前に戻す）を 1 本で確かめる．
3. **修正 3 のテストは規則ごとに 3 本に分けた**．self-review の指摘で P240・P241 を足し，上限（10 ms）と待ち時間（timeout）の形も固定した（brief の「規則ごとに 1 行」への上乗せ）．
4. **`subdev_spec_lifecycle` は残した**：`tests/uhd_api.rs` の `uhd_api_a_refused_subdev_spec_is_freed_once` が使い，実 libuhd の拒否の文面と経路を pin している（free の回数は ffi テストが見る）．
5. **self-review の指摘で足した直し**：
   - `streamer_lifecycle` の「二度目の free は必ず abort」も同じ誤りだったので直した（UHD の streamer の free も NULL を入れる：`usrp_c.cpp:97-101`，`:153-157`．コピーの free は abort する）．
   - 1b-4a の文の「a refused reopen leaves the stream open for nothing」を「leaves the stream with none」に揃えた（`design/18`，`device.rs`，`fake.rs`）．
   - `plan/phase8/33-channels-and-streams.md` の「`UhdRadio` の挙動は変えない」に，修正の回 1 が変えた 0 channel の例外を書いた：HEAD の `UhdDevice` の `tx_send` は 0 channel の streamer への 1 本の buffer を（`samples.len() != stream.channels` で）C の前に拒否していたが，UHD の `sanitize_stream_args` はその列を `{0}` と読むので，今は channel 0 の streamer として C に届く．実機は BP0，pin テストは未了（§9）．
   - 引用の行番号を直した：`subdev_spec.cpp:44-56` → `:58-59`（`default:` 58，`throw` 59），`subdev_spec_c.cpp:38` → `:41`（`(*h) = NULL`），`db_basic_and_lf.cpp:107-117` → `:101-111`（名前を決める switch）．docker のイメージの UHD 4.10 のソースで確認した．

## 4. 確かめられず INFERRED にした UHD の挙動

- 置き換えた ffi の関数は，docker のイメージの UHD 4.10 のソースを読んで模した：make は先に handle を `new` してから解析し，失敗しても handle を残す（`subdev_spec_c.cpp:31-36`）；free は delete の後に NULL を入れる（`:39-41`）；3 分割以上の token は `value_error`（`subdev_spec.cpp:58-59`）．**ソースの読みで，実行での確認ではない**．テストが数えるのは free の回数である．
- 「channel map から外れている間に期限が来た timed command を UHD の radio block がどう扱うか」は未測定（fake のモデル．UR-33 の文・Changes 行・plan 33 §5 に INFERRED と明記）．BP0 に残す．
- 修正の回 1 の INFERRED（pre-chain の rate，空 spec・重複の拒否，同じ channel の二重 open，`serial`，front end の設定の保持，`UhdDevice` の新しい経路）は変わらない．

## 5. 規則ごとのテストと mutation の行

| 規則（spec 18） | テスト | mutation の行（誤りの形 → kill するテスト） |
|---|---|---|
| spec の変更は，期限の来た予約を離れる側の spec で settle する（UR-33） | `ur_33_a_command_due_while_its_front_end_was_out_never_applies`（T より後に戻すと捨てられて既定値／前に戻すとその front end に T で効く） | P236 `self.settle(&mut st);` を外す → 同テスト（期限切れの予約が戻った B に 2 GHz で効く） |
| streamer のない stream の `rx_packet_samples` は 0（UR-33） | `ur_33_a_packet_size_read_of_a_stream_with_no_streamer_is_zero`（`rx_packet: Some(1_996)` の fake：streamer なしで 0，開いた後 1 996） | P237 `contains_key` の検査を外す（1 996 を返す）→ 同テスト |
| streamer のない stream の `rx_recv` は最大 10 ms 待って `Failed`（UR-33） | `ur_33_a_recv_on_a_stream_with_no_streamer_waits_its_10_ms`（manual clock で待ちがちょうど 10 ms） | P238 sleep を外す（待ち 0）／P240 `timeout` をそのまま待つ（待ち 60 ms）→ 同テスト |
| streamer のない stream の `tx_async` は timeout 待って `Ok(None)`（UR-33） | `ur_33_a_report_read_on_a_stream_with_no_streamer_waits_its_timeout`（待ちがちょうど 60 ms） | P239 sleep を外す（待ち 0）／P241 10 ms だけ待つ（待ち 10 ms）→ 同テスト |
| `set_subdev_spec` は handle を各経路で 1 回だけ解放する（UR-3） | `uhd_a_subdev_spec_is_freed_once_on_every_path`（feature `uhd`：成功・解析の失敗・set の失敗で make 1・free 1） | mutation の行はない（feature `uhd` のテストは gate でビルドできない：§9） |

## 6. 当て直した行

なし（この回で `old`/`new` を書き換えた既存の行はない．全 6 リスト 1000 行の `old`（`old2` を含む）が，それぞれのファイルでちょうど 1 回一致することを機械的に確認した）．

## 7. 確かめること 1〜6 の結果（最終ツリー，最後の編集の後に同じツリーで実行）

| # | コマンド | 結果 | ログ |
|---|---|---|---|
| 1 | `cargo test --workspace --no-fail-fast` | 65 suite すべて ok，1279 passed / 0 failed | `check1-fix2-final.log` |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` | warning/error 0 | `check2-fix2-final.log` |
| 3 | docker（`ezsdr-v4-dev:uhd4.10`）: clippy `--features uhd --all-targets` → `--lib` → `--test uhd_api` | clippy 0，lib 111 passed / 0 failed（新しい ffi テストを含む），uhd_api 5 passed / 0 failed | `check3-fix2-final.log` |
| 4 | `cargo build -p ezsdr-server` → `PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=… ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests` | 47 tests，OK | `check4-fix2-final.log` |
| 5 | `PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base HEAD` | 0 problem | `check5-fix2-final.log` |
| 6 | `… remote_mutate.py maintenance sim02:8,sim03:8 P208 P209 P210 P229 P232 P233 P234 P235 P240 P241` → 最後に `all sim02:8,sim03:8` | 10/10 killed；`all` は 999/1000 killed（maintenance 624/624，phase3 83/83，phase4 34/34，phase5 29/29，phase6 63/63，phase7 166/167）．生き残りは既知の `phase7/G09` のみ（SURVIVED．`fix2-brief` の「既知の生き残りは phase7/G09 だけ」と同じで，この段階の変更とは無関係） | `check6-rows-fix2-final.log`，`check6-all-fix2-final.log` |

## 8. セルフレビュー（AGENTS.md §9 手順 2，Haiku 5.5 `anthropic/claude-haiku-5-5#high`，read-only，3 回）

- **1 回目**（`fix2-selfreview-1.log`）：P0 なし，P1 1 件（解放の規則を殺す mutation の行がない：feature `uhd` は gate の外），P2 5 件（`rx_recv` の上限と `tx_async` の待ち時間の形の行がない／settle の文に INFERRED がない／`tx_send` の「at once」が fault のない場合に限る／ffi の make の判定が UHD と違う／`rx_packet_samples` の Provider への影響は全テストで確認を）．直した：P240・P241，INFERRED の注記，fault の限定，判定を `count() > 2` に，全ワークスペースのテスト（1279 passed）．P1 は報告に明記（§9）．
- **2 回目**（`fix2-selfreview-2.log`）：P1 3 件（0 channel の扱いが報告にない＝修正の回 1 の記録で，plan と Changes 行と fix-report §3-3 にあること，`UhdDevice` 側の pin テストが未了であることを確認／証跡が最終ツリーより古い／`uhd.rs` の規則が gate の外）と P2（引用の行番号，§6 の行，`split(' ')` の確認，`rx_packet_samples` の注記，文言）．直した：引用 3 件の行番号，文言は 3 回目で対応．P1 は §9 と報告本体に反映した．
- **3 回目**（`fix2-selfreview-3.log`）：P0 なし，P1 1 件（plan 33 の「`UhdRadio` の挙動は変えない」が 0 channel の例外と矛盾），P2 5 件（`db_basic_and_lf.cpp` の引用／`uhd_api.rs` の streamer 版の「double free aborts」／「open for nothing」は HEAD でなく 1b-4a の文／§6 の数の説明／`subdev_spec_lifecycle` を残す理由と報告の未了），P3 3 件．直した：P1（plan 33 に例外を追記），P2 の 1〜3（引用 2 件と文言）．P2 の 4・5 と P3 は §9 に明記し，採用しなかった P3 の理由も §9 に書いた．

## 9. 未解決のこと（1b-4b・1b-4c・owner へ）

- **feature `uhd` の規則は mutation の gate の外**：`mutate.py` の `cargo_test` は feature を渡さず，mutation の image（`plan/maintenance/tools/remote/Dockerfile`）は rust:1-bookworm で libuhd も UHD の header も持たないため，`uhd.rs` を対象にした行は gate でビルドできない．gate の外にある `uhd.rs` の規則：subdev spec の handle の寿命（make の失敗時の free，set の失敗時の free，成功時の free），streamer の map と `close_streams`，空の channel 列の channel 0 への正規化，`serial`・`rates`・antenna の read-back，UHD の C API との型の突き合わせ（`uhd_api_struct_sizes_match_the_headers`）．確認は確かめること 3 の docker 実行（`--features uhd --lib` と `--test uhd_api`）で取っている．**orchestrator へ**：handoff に「feature `uhd` を mutation の gate に入れる（image に libuhd と header を入れ，行に feature を持たせる）」を立てるのがよい．
- **0 channel の `UhdDevice` の経路に pin テストがない**：`tx_open(0, &[])`／`rx_open(0, &[])` が channel 0 を C に渡すこと（修正の回 1 の変更）は，fake では `ur_33_an_empty_channel_list_is_channel_zero` が pin しているが，`UhdDevice` 側は実機か置き換えた ffi テストが要る．BP0 で実機を見る．
- **`ur_33_a_burst_resumed_after_an_underflow_plays_late` など，§6 の表に行のない `ur_33_` のテスト**：HEAD は 16 本中 7 本，今は 32 本中 10 本．増えた 3 本は今回足した境界の unit test で，§6 の `src/device.rs` の段落と UR-33 の `*Test:*` 行に列挙してある（表の行は既存の 7 本と同じく持たない）．
- **採用しなかった self-review の P3**：`rx_streamer_impl.hpp` の引用はパスを略したまま（HEAD の文で，ファイル名は一意），`multi_usrp_rfnoc.cpp:2796` の範囲もそのまま（関数の先頭を指す．fix の回 1 の記録），`provider/tx.rs:485` の行番号（計画の文面の指摘で，コードの引用ではない）．
- **1b-4c**：untimed の antenna 読み戻し（D16）はまだ誰も読まない．読み戻しの失敗で `apply` が失敗する経路の扱い．spec の設定順序，`opened`（`rates` の pre-chain の履歴）の spec 変更での消去の規則．
- **1b-4b**：`UhdRadio` の呼び出しは stream 0，`Scripted` は stream 番号を無視，channel 数を 0 にする cold change は `tx_open(0, &[])` を呼ぶ（段階の report §9 と同じ）．
- **BP0（bench）**：段階の report §9 の項目（pre-chain の rate，reconnect の文面，`serial`，front end 名，antenna の読み返し）に，今回の「channel map から外れた front end の期限切れの command」と「0 channel の `UhdDevice` の経路」を足す．
