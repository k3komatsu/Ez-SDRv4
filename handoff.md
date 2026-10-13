# Ez-SDR v4 — Handoff (2026-10-14)

次のセッション（人間・AI どちらでも）が最初に読む現状メモ．**今のことだけ**を書く：どこに何があるか，何が未解決か，owner の判断待ち，次に何をするか．終わった作業の記録はその置き場（`plan/phaseN/00-overview.md` か各計画ファイル）に書き，ここには1行の参照だけ残す（AGENTS.md §9．`rules.py` が 300 行・40 000 byte を超えると落ちる）．2026-10-14 までここにあった記録も同じ日に各置き場へ移した．

開発の恒常的なルールは [AGENTS.md](AGENTS.md)．

## 1. 今の状態

- **Phase 1〜7 は Gate X 受理済み**．Phase 7（UHD Provider `ezsdr.radio.uhd`）は 2026-10-01 に X300＋OBX で bench B0–B9 を通してクローズ（[plan/phase7/00-overview.md](plan/phase7/00-overview.md) §11，[bench-results.md](plan/phase7/bench-results.md)）．accepted specs の一覧は AGENTS.md §1・§2．
- **凍結前の改修（項目1〜8）は完了**（[plan/maintenance/24-prefreeze-audit.md](plan/maintenance/24-prefreeze-audit.md)）．最後の項目8（spec 01〜06 を DA-1 形式に）は 2026-10-14 に完了（[計画31](plan/maintenance/31-item8-kernel-specs.md) §10）．その前の保守の記録は各計画ファイル（Phase 7 クローズから spec 20 までは [audits-2026-10.md](plan/maintenance/audits-2026-10.md)）．
- 実装：13 crate（`crates/`），schema（`schemas/`），Python package（`python/ezsdr`）．確認の手順は §5．
- open issue は4件：#51，#56，#59，#62（§3）．

## 2. 次にすること

1. **Phase 8（Mock ↔ X310 parity）の計画（実機なしで Gate P まで進める）**．`plan/phase8/` はまだない．
   - Vision §59 の parity test：測った envelope と Mock profile を比べ，Mock の方が緩ければ失敗．入力は [plan/phase7/00-overview.md](plan/phase7/00-overview.md) §3「Phase 8 inputs」（当時の値．旧 restart model を含むので現行の accepted specs と照合する），NB-5，UR-32，spec 18 の lead の値，§3 の H1〜H5 と B8 の未測定項目．
   - 測定テストと比較の道具を先に書き，FakeDevice でリハーサルしておく（実機の時間は流すだけにする）．`hardware.rs` には `hw_b8_*` が約20本ある．H1〜H5 は未実装．
   - 計画で最初に owner に諮ること：Mock の比較相手（§3 の1行目）．続いて #62・#59・#51 を実機の前に直すか Phase 8 の中でやるか．判断は1件ずつ利点・欠点・代案つきで．
2. **実機は owner が準備中**（2026-10-14：「全部準備できます」．時間がかかる）．owner が揃ったと言うまで実機は動かさない．各段階で要るもの（根拠：Vision §67〈`design/vision/11-boundaries-invariants-and-roadmap.md:317-337`〉，`plan/phase7/00-overview.md:103`「One device is enough for §59's claim」）：
   - **8**：1台．Phase 7 と同じ X300（X310 の代わりで可）＋OBX 1枚のループバック（TX/RX → 30 dB 以上の減衰器 → RX2），10 GbE の Linux PC．外部 10 MHz/PPS 不要．X300 は 2026-10-01 から未接続なので，つなぎ直して `uhd_usrp_probe` で確認する（§6）．
   - **8b**：USRP 2台（2台目は X310/X300 なら同じ driver と profile．USRP2 は不要〈owner〉），10 MHz/PPS の分配器（OctoClock など．X3xx の背面から配れるかは未確認），2台の時刻差を測る RF 配線（1つの信号を分配して両方の RX へ，減衰器付き．推測）．2台目の 10 GbE はサーバーの X710 port 1（`enp2s0f1np1`，今は未使用）が使えそう（未確認）．
   - **8c**：8 と同じ1台．FPGA image に Replay・DDC/DUC（・FFT）の block があるかを `uhd_usrp_probe` の block 一覧で確認．
3. **その後の順番**（owner 2026-10-10，Vision §67，[28-forward-audit.md](plan/maintenance/28-forward-audit.md) 判断7）：8 → 8b（複数台：1 instance・1 root，判断1．`Session.aligned(at)`・`next_pps(k)`・`EpochRef::Pps` は実装済み，U12 は 8b）→ 8c（RFNoC：Replay，DDC/DUC．device FFT は FPGA image に block があるときだけ．UHD の RFNoC API は C++ のみなので UHD Module に薄い C++ 層が要る）→ 10（Processor/Reactor の実行エンジン，最小の PDU 経路）→ 全規則の確認 → 凍結レビュー → **v4.0 凍結** → 11+（Packet/PDU + TUN/TAP＝旧 Phase 9，WASM，GPU…）．

## 3. 未解決と owner の判断待ち（一覧はここだけ）

| 項目 | 中身 | いつ・どこで |
|---|---|---|
| Mock の比較相手 | MockRadio の `x310-like` は UBX の値．OBX 用の Mock profile を作る（機材と1対1，profile が増える）か，UBX を1枚用意して `x310-like` と比べる（profile は増えない，OBX の Phase 7 の結果は流用できない） | Phase 8 の計画の最初（owner） |
| `radio` 語彙の phase `random` | `x310-cbx` の制約（[design-notes](plan/phase7/design-notes.md) §10）．OBX を使う限り不要 | Phase 8（owner） |
| #62 | MockRadio が UHD の device queue の順番どおりの release を模擬しない（Mock が実機より緩い）．VH-8 harness は `timed` の25系列を除外中 | Phase 8 の計画で |
| #59 | MockRadio に「cold change の read-back 拒否」の fault がない | Phase 8 の計画で |
| #51 | 送信 Stop で held burst を捨てるとき UHD は `COMMAND_REJECTED` を出し MockRadio は黙る．先に「イベントの報告規則も1か所で実装するか」を決める（`mech:radio-rule-implemented-twice`） | Phase 8 の計画で（owner） |
| #56 | X310 で start の e₁〜e₂ 間の Stop と Stop の処理遅れを測る | Phase 8 の bench |
| H1〜H5 | H1：開始が遅れた複数 packet の burst．H2：burst 途中の Underflow（再開時刻，ずれ）．H3：TX の `max_num_samps`．H4：負荷下の uhd-tx・uhd-control の起床遅延．H5：保持した最後のサンプルの 2 ms 期限 | Phase 8 の bench |
| 方向別の start lead | owner 決定（2026-10-08）：`radio.timing.start_lead_ns` を TimingEnvelope の方向別の値にし，送信側は Phase 8 で測る（それまで 50 ms）．**未実装**：今は単一値（`design/07-radio-model.md:85`，`crates/ezsdr-radio/src/lib.rs:576`） | Phase 8 の計画 |
| 測って決め直す値 | 送信と受信の start lead（今は 50 ms，INFERRED），cold change・再開で受信が途切れる時間（約55 ms〜221 ms），UR-17 の受信要求長の上限（残すと決定済み，実測で見直す） | Phase 8 の bench |
| 保守表の 3・4 | 3：fake は TimeError で burst 全体を捨てるが FPGA の既定は packet 単位（H1 の後に UR-33 と fake を直すか）．4：保持した最後のサンプルの 2 ms 期限（設計の代償として受け入れるか見直すか）．表は [audits-2026-10.md](plan/maintenance/audits-2026-10.md) の「Phase 7 後の保守」（1・2・5 は解決済み） | Phase 8（owner） |
| Provider の id を名乗る写し | 受け取り時の拒否で id を名乗る処理が MockRadio・UHD に約15か所ずつ（MR-18・UR-26 の一部は未固定）．1か所で id を渡す形にまとめると減る | 任意 |
| 規則ごとの確認への記録 | TM-21 の budget/block period 比較は未実装．TM-16c の既存テストは ManualTimeAuthority で paced Authority の規則を固定しない．SC-29 の SOB なし time jump の `requested_target` 不在と SOB 付き time jump の組合せは未固定．Authority の `now/schedule/cancel` の panic の扱いは規則との対応を要確認 | 凍結前の全規則の確認（§4） |
| 負荷に弱いテスト | maintenance/J293 の無変異テスト `ur_26_the_shutdown_s_last_burst_ack_waits_on_the_device_s_clock` が16 shard の負荷下で1回失敗（2026-10-14）．再発したら #60 の続きとして調べる．Python の `test_v54_sleep_is_run_time` が Mac で 10 s の上限をわずかに超える記録（2026-10-07，その後は未確認） | 再発時 |
| server の小さな文言 | `Ran.path` の doc と schema の説明に「書けなかったときは無い」を足せる（第6監査） | 任意 |
| 後の phase へ | Session replay と artifact store（frontend）．Phase 10 へ（Phase 5 から）：event edge，component parameter を適用する Executor（UC-2〜UC-6，MA-24），component の budget を仮想時間で課す，MA-30 の Action latency．Phase 2 の test ceilings は [implementation-notes.md](plan/phase2/implementation-notes.md) | 各 phase |
| GitHub Pages | https://k3komatsu.github.io/Ez-SDRv4/ の状態表示は 2026-10-11 時点の実装と 2026-10-01 の実機確認まで反映済み．更新は owner の依頼があるときだけ（AGENTS.md §1） | owner |
| v3 → v4 の残り | (1) ghcr の image 名 `ezsdr:latest` を v3 と取り合う：v4 の名前を変えるか v3 利用者を固定 tag へ．(2) v3 readme の `k3kaimu` と現 owner `k3komatsu` の不一致（未確認）．(3) v3 の readme に `Ez-SDRv4` への1行 | v4 リリース前（owner） |

## 4. v4.0 凍結のチェックリスト（残り，Phase 10 の後）

済んだ項目（項目1〜8，F19・F21）と根拠は [24-prefreeze-audit.md](plan/maintenance/24-prefreeze-audit.md)．凍結レビューは次の全項目が済んでから：

- [ ] spec 01〜06 の全規則（314）と OV 規則を外部の安いエージェント（DeepSeek V4.1 Flash）で1つずつ確認：3つの決まった問い，JSON，根拠必須．Claude は印の付いた行を検証し，残りを抜き取り．機械で確かめられる分はスクリプトで先に（[28-forward-audit.md](plan/maintenance/28-forward-audit.md) 末尾）．§3 の「規則ごとの確認への記録」もここで．
- [ ] MA-16a の公開 API 許可リスト（`kernel_surface_allow.txt`）の見直し．
- [ ] 文書の種類ごとに受け付ける版を設定する（今は共通の `SUPPORTED_VERSIONS`．項目5で凍結時に延期，[24-prefreeze-audit.md](plan/maintenance/24-prefreeze-audit.md) の 2026-10-09 item 5 の行）．
- [ ] Kernel API を 4.0.0 に，`SCHEMA_CHANGELOG.md` の見出しを凍結後の形式に．凍結までは版を上げず changelog も書かない（AGENTS.md §6）．
- [ ] 凍結後の OV-12 を道具で守る：`EZSDR_UPDATE_SCHEMAS=1` は形の変わった `*.v1.json` を書き換えず拒否する（説明文の修正だけ許す）．

## 5. 進め方と道具

- **分担**（AGENTS.md §9）：Claude Opus が回す → 実装役（DeepSeek V4.1 Flash，OpenCode2．GPT-6 Luna は owner の指定時だけ）が実装し，自分でセルフレビューを最大3巡 → Opus がレビュー → Sol 6.1（OpenCode2，読み取り専用）が独立にレビュー → Opus が採否を決め，実装役が直し，gate → Opus が commit・push．Sol は owner の Codex 枠を使う：`python3 plan/maintenance/tools/codex_limits.py` で残りを見る．
- **実装役への指示書に毎回書くこと**：mutation は `remote_mutate.py` だけ（Mac の `mutate.py` も作業ツリーへの mutant も禁止，実験はリポジトリ外のコピーで）．`cargo fmt`/`rustfmt` 禁止．Mac で負荷をかけない．セルフレビューはフォアグラウンドで終わるまで待つ．DeepSeek のツールは1コマンド120秒で打ち切るので，長い処理（`rules.py --sentences` 約5分など）は時間を延ばすよう書く．前の段階で出た誤りの型は禁止として列挙する（8g で効いた）．
- **gate**（段階の最後）：
  - mutation 全リスト：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py all sim02:8,sim03:8`（約8分）．既知の生き残りは phase7/G09．結果が欠けた行は ID を並べて再実行（zsh では `$ids` が分割されない）．phase7 G16 と phase6 G21 は timeout で kill が正常．shard × 4 CPU は1ホスト32コア以内．
  - 手元：`cargo test --workspace --no-fail-fast`，`cargo clippy --workspace --all-targets -- -D warnings`，`cargo clippy -p ezsdr-radio-uhd --features uhd --all-targets -- -D warnings`，Python `PYTHONDONTWRITEBYTECODE=1 EZSDR_SERVER=~/.cache/cargo-target/Ez-SDRv4/debug/ezsdr-server ~/.cache/ezsdr-venv/bin/python -m unittest discover -s python/tests`．
  - 文書：`PYTHONDONTWRITEBYTECODE=1 python3 plan/maintenance/tools/rules.py --base <変更前の commit>`（`--self-test` で道具自体を確認）．
- **見張りと機械チェック**（モデルなし）：
  - `plan/maintenance/tools/mutation_watchdog.py`：Mac の `mutate.py`・`cargo fmt`/`rustfmt`・作業ツリーに当たった mutant・親のいない／20分を超える test を見つけたら終了して知らせる．実装役を動かす間バックグラウンドで流し，gate を閉じたら止める．判定は実行中のスクリプト名で（文字列一致は誤検出した）．
  - 段階ごとの道具（`~/.cache/ezsdr-fixes/2026-10-13-item8/` の `check05.py`・`drive04.sh`・`watchdog-04.sh` が雛形）：指示書の機械で確かめられる項目を検査し，未達を新しい実装役セッションへ最大3回差し戻す．**起動前に `--calibrate` などで誤検出がないことを確かめる**（8f では誤検出3件で差し戻しが空回りした）．
  - セルフレビューのレビュー役が API 待ちで固まることがある（8g で約20分，CPU 0%，ログが進まない）：ログの更新時刻を見て，レビュー役だけを止める．
- **sim02/sim03**：`ssh sim02`．直接つながらないときは owner の `~/.ssh/config` が `ydev` を踏み台にする（トンネル不要）．
- **owner の方針**（AGENTS.md §6）：研究用なので大げさにしない，凍結まで破壊的変更は自由で版管理しない，簡素化はテスト・レビュー・mutation で制御を保つ，削除・統合を好む，判断は1件ずつ利点・欠点・代案つき，返答は日本語．同じ失敗機構の3件目では設計を疑う（[failure-mechanisms.md](plan/maintenance/failure-mechanisms.md)）．

## 6. 実機環境（Phase 8 で使う．2026-10-01 時点）

手順の本体は [plan/phase7/bench.md](plan/phase7/bench.md)（B0–B9，RF 安全，合格条件），結果は [bench-results.md](plan/phase7/bench-results.md)．

| 項目 | 値 |
|---|---|
| サーバー | `usrp-lnx02`（Ubuntu 24.04，x86_64，20 core）．`ssh usrp-lnx02`（port 10022，user `komatsu`）．sudo はパスワードが要るので owner に頼む．clone は `~/works/Ez-SDRv4`（作業前に `git switch main && git pull --ff-only`．Phase 7 の頃は `worktree-phase7-impl` だった） |
| Rust / UHD | ホストには Rust がなく UHD は apt の 4.6（OBX を知らない）．すべて Docker image `ezsdr-v4-dev:uhd4.10`（`docker/uhd4.10/Dockerfile`，UHD 4.10，Rust stable と 1.85.0）の中で．build 出力は volume `ezsdr-v4-cargo-target` |
| 装置 | USRP X300（serial 347D545），FPGA は UHD 4.10 の HG image（もう書き換えない）．slot A に OBX，slot B は空（2 + 2 channel と出るのが正常）．profile は自動で `x310-obx` |
| RF | OBX の TX/RX → 30 dB 減衰器 → RX2．利得 0 dB，振幅 ≤ 0.5，1 GHz（envelope 999–1001 MHz） |
| ネットワーク | X300 の 10 GbE ↔ X710 port 0 `enp2s0f0np0`（`192.168.40.10/24`，MTU 9000）．**`ARGS=addr=192.168.40.36`**．2026-10-01 のクローズ時は carrier 0（未接続） |
| 既知の癖 | ref PLL の lock 失敗が open の約4%（UR-7 の1回の開き直しで救える）．X300 は受信の timed stop を無視する（F1，対処済み）．抜線で UHD が static teardown で abort（UR-29） |

実行の形（`-w /work` を忘れない）：

```sh
cd ~/works/Ez-SDRv4
docker run --rm --network=host --cap-add=SYS_NICE --ulimit rtprio=99 --user "$(id -u):$(id -g)" \
  -v "$PWD:/work" -v "$HOME/ezsdr-bench:/bench" -v ezsdr-v4-cargo-target:/cargo-target -w /work \
  -e EZSDR_UHD_ARGS=addr=192.168.40.36 ezsdr-v4-dev:uhd4.10 \
  cargo test -q --release -p ezsdr-radio-uhd --features uhd --test hardware <test> -- --ignored --nocapture \
  > ~/ezsdr-bench/<test>.log 2>&1
```

装置の確認：`docker run --rm --network=host ezsdr-v4-dev:uhd4.10 uhd_usrp_probe --args addr=192.168.40.36`．長い出力はファイルに書いてから読む．リンクが上がった直後の `uhd_find_devices` は空振りすることがある．実機で失敗したらその場でコードを直さず，記録 → FakeDevice で再現するテスト → 修正の順．

## 7. リポジトリと記録の索引

- branch と remote は AGENTS.md §1．`origin` = `k3komatsu/Ez-SDRv4`（`main` に push）．v3 は `k3komatsu/Ez-SDR`．**`git push --tags`/`--all` をしない**．
- ローカル：`~/work/Ez-SDRv4`（`main`），`v3/`（`master` の git worktree，消さない．設計文書が `v3/...` を22件引用），`/Users/komatsu/orca/workspaces/Ez-SDRv4/gh-pages`（`gh-pages` の worktree）．`v3/` を消したら `git worktree prune && git worktree add v3 master`，リンクが切れたら `git worktree repair v3`．引用の確認：

  ```bash
  grep -rhoE 'v3/[A-Za-z0-9_./-]+' design plan Ez-SDR_v4_ARCHITECTURE_VISION.md | sort -u | while read p; do test -e "$p" || echo "MISSING $p"; done
  ```

- 記録の置き場：
  - 各 Phase：`plan/phase1/` 〜 `plan/phase7/` の `00-overview.md`（決定ログと governance）．
  - 保守：`plan/maintenance/20-*.md` 〜 `31-*.md`（spec 20〜22 の修正，設計メモ 23・25，凍結前監査 24，spec 26・27，前向き監査 28，項目3 の 29，#60 の 30，項目8 の 31）．失敗機構の一覧は [failure-mechanisms.md](plan/maintenance/failure-mechanisms.md)．
  - UHD spike（branch `spike/uhd`，merge しない）：[plan/spikes/2026-09-26-uhd.md](plan/spikes/2026-09-26-uhd.md)．
  - 実装役の報告・レビュー・gate のログ：`~/.cache/ezsdr-fixes/<日付>-<作業>/`（リポジトリ外，消えうる）．
  - Phase 7 クローズ（2026-10-01）から spec 20 までの保守と監査（#1〜#38）：[audits-2026-10.md](plan/maintenance/audits-2026-10.md)．
  - 2026-10-14 までの handoff の全文は git の履歴（`git show 60c7494:plan/handoff-archive.md`）．
