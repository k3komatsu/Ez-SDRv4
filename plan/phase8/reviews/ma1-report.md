STATUS: done

# MA1 報告（実装役，2026-10-28）

## 作ったもの
- `ezsdr_radio::timeline::covering`（RM-36，`timeline.rs`）と unit test `rm_36_covering_boundaries`。UHD の `reach` は `covering(..).is_some()` を呼ぶだけに置換（振る舞い不変）。
- Mock：`PendingUpdate.timed_from`（受けた時に覆った stream と origin），`covering_rx`（link 有りの受信 stream だけ走査），適用時に同じ segment なら timed，他は untimed（`apply_layout_channel_update`）。送信は不変。hook は受け取りと適用の 2 か所だけ。
- 試験（mock_layout.rs）：計画の 1，1b，2，3，4，5b，6，7，10a，10b，11〜16，17a，17b，18，19 と，足した送信の試験 1 件。計画の名前どおり。
- spec：design/07 RM-36（§6 行，Changes），design/09 MR-34 の段落・Ceiling 3 つ・Test 行・§6 の改名行・MR-27 の 1 行・Changes，failure-mechanisms.md に 1 段落。design/18，KNOWN_LIMITATIONS.md は触っていない。

## 計画から変えた所と理由（詳細は ma1-test-plan.md 末尾「実装で足したこと」）
- RM-ID は RM-36（RM-35 は使用済み）。
- 1b：deterministic は `ideal` だけで別 stream に置けないので `ideal` で位相 0・coherence deterministic を観測。6：CBX の 2 stream で更新ごとに 1 回引くことを観測。
- 10a：timed の calibration でも S2 の origin tune が retune[0] を引くため再開の引きでは区別できない。calibration は S2 の coherence と位相で timed を示し，fault 側は再開が retune[1] で untimed を示す。
- 7：tx を 2 stream にした（c1 の受信を聞くため）。5b：Stop に instant が無いので「更新→stop，e=cut」の形。
- 試験 2，11，16 の場面は計画と少し違う（2：T0−1ms の更新の無い run，11：burst は tx sample 2_000_000，16：e=T0+150ms）。観測する性質は同じ。
- 足した試験 `mr_34_a_layout_a_transmit_origin_s_untimed_tune_follows_a_timed_update_at_its_instant`：既存 mutation P187（untimed tune の rank）は rx の旧試験だけが殺していて，MA1 で rx では区別できなくなるため，送信側で P187 を殺す。

## 変えた既存の試験
- `mr_34_a_layout_untimed_tune_follows_a_timed_tune_at_its_instant` → `mr_34_a_layout_an_untimed_update_and_the_entry_tune_at_one_instant_draw_in_application_order`。更新は origin より前に受けるので untimed：更新が retune[0]，origin の tune が retune[1]，origin 以降の位相は retune[1]。理由：新しい規則で covered な更新は origin と同じ instant に来られない。`--renames`：`/home/komatsu/.claude/jobs/8aebdd6c/tmp/renames.txt`。他に落ちた既存の試験は無し（workspace 全部 pass）。

## mutation
- 新規 P999〜P1013（M1〜M14，M16）：すべて killed。
- 直した既存の行：P168，P589，P965，M63（old の text 変更），P967/P968/P969/P989（判定が timeline.rs に移動），P187（filter を送信の新試験へ）。
- 流した行（maintenance,phase3,phase7，54 行＋P187/P989 再実行）：P158〜P160，P168，P170，P175〜P178，P181〜P183，P186，P187，P190，P191，P193〜P198，P589，P965〜P970，P973，P989，P999〜P1013，M31〜M33，M46，M55，M59，M63，U21。最初の実行で P187，P989 が SURVIVED，上の直しの後の再実行で 2/2 killed。
- **UHD の試験の隙間（要判断）**：`reach` の `holds`（`next` の選び方だけに使う）の channel の絞りを外しても UHD のどの既存の試験も落ちない（以前は `passes` 経由で殺していた）。P989 は covering の絞りに移して killed。`next` 側の絞りを見る UHD の試験は足していない（約束）。

## 確かめたこと
- `cargo test --workspace --no-fail-fast` 全 pass，`cargo clippy --workspace --all-targets -- -D warnings` clean，`git diff --check` clean，`rules.py --base a9b0a26 --renames ...` 0 problems。純粋なテストは 1 回。

## セルフレビュー（1 回，free は上限で失敗し opencode-go/deepseek-v4.1-flash#max で実施）
- P1-1 mutation ID が 3e の P999〜P1017 と衝突：**merge 時の作業**として orchestrator に残す（MA1 の P999〜P1013 を振り直す。tests/計画の記録も同じ番号）。
- P1-2 MR-34 の untimed の文が `random` を coherent でないと読ませる／gain も含む：**直した**（frequency update に限り，coherence の文を `random_unless_timed_tune` だけに）。
- P3 MR-27 の「every applied update」：直した（frequency update）。Test 行の区切り：225 行にコンマを足した（222 行は元から and あり）。`lo_draws` の doc：直した。
- P3 1b3c-fix-reports.md の P187 の killer が古い：**退けた**（凍結された記録。計画末尾に移動先を記録した）。
- P3 計画と場面が違う 3 点：上の「計画から変えた所」に書いた。

## 残ったこと
- 3e との merge：mutation ID の振り直し，design/18 UR-24 の「覆う」の文を RM-36 の引用に替える，KNOWN_LIMITATIONS.md の MA-1 の項目（Mock 側の保守的な Ceiling は design/09 にある）。
- 新規 record link：design/07・09 の Changes の record 欄は「Phase 8 stage MA1」の文字だけ（記録ファイルが無いため）。orchestrator が plan/ に記録を置いて link する。
- red（HEAD で落ちること）は revert して確かめていない。mutation の killed が代わりの証拠。

---

STATUS: done

# MA1 修正 1：規則の差し替え（実装役，2026-10-28，main の tree）

## 変えたもの
- Mock（`crates/ezsdr-mock-radio/src/lib.rs`）：`PendingUpdate.timed_from` と受け取りの配線を削除。`apply_layout_channel_update` は適用の時に `covering_rx(number, e)` の origin が `e − W` 以下なら timed。W は `self.profile.timing().release_window_ns` を `time::ns_to_v` で root tick に直したもの（値は写さない。変換に失敗したら W = i64::MAX で untimed）。送信，UHD は不変。
- 試験（mock_layout.rs）：timed に変えた 4 本（名前も変更）：
  - `..._an_update_received_while_stopped_is_timed_when_a_later_segment_covers_e`
  - `..._a_flip_flop_is_timed_by_the_segment_that_covers_e`
  - `..._an_update_received_before_t0_is_timed_when_its_segment_has_run_the_release_window`
  - `..._an_update_due_in_a_refusal_advanced_segment_is_timed`（旧 17a）
  それぞれ timed の定数，coherence valid，更新の引きが無いこと（restart / 起点の tune が retune[0]）を assert。
- 算術の試験を `mr_34_a_receive_update_is_timed_from_the_release_window_after_the_segment_origin` に置換：e = T0 + W は timed，W − 1 tick は untimed，T0 前の受け取り，Session の 2 ms lead（e = T0 + 2 ms は untimed），T0 + 5 ms の受け取り（timed）。W は description から読む。
- Spec Run の regression：`mr_34_a_scheduled_receive_retune_inside_a_running_segment_is_a_timed_tune`（`crates/ezsdr-acceptance/tests/v58.rs`，scheduled `update_parameter`，捕捉 sample の位相と coherence）。
- 受け取りに基づく comment を直した（:1743 ほか）。
- **計画に無い直し**：`crates/ezsdr-mock-radio/tests/mock_channel.rs` の `mr_32_b_the_transmitted_timelines_...` が落ちた（更新の e が origin の 2.5 ms 後 = 窓の内側で untimed になる）。試験の目的は送受の読み出し instant なので，更新を T0 + 3.5 ms に動かした（k の境界と run_to，comment も合わせた）。
- mutation：P1022〜P1025，P1029〜P1035 は残した。P1026〜P1028 を「窓の判定を落とす」「等号を拒む」「適用の時の覆いを省く」に，P1036 を「窓を無視（0）」に置換。P1037（受信の更新は常に untimed，Spec Run の regression が殺す）を足した。P589 の old を `timed_from` 無しに戻した。
- spec：design/09 MR-34 の規則の文と受け取りの Ceiling を窓の規則と「W 未満の (e − W, e] の保守的な例外」に替え，前提の Ceiling，引きの Ceiling，位相と引きの規則，送信は残した。Test 行と Changes を直した。design/07 RM-36 と design/09 の Changes の record 欄を `[plan/phase8/reviews/ma1-opus.md](../plan/phase8/reviews/ma1-opus.md)` に。design/18 UR-24 に RM-36 の引用。KNOWN_LIMITATIONS.md KL-32 を MA1 の後の事実に（Symptom，Workaround に「origin から W（3 ms）以上後」，Status）。
- 計画 `ma1-test-plan.md` 末尾に「規則の差し替え（2026-10-28）」を足した。

## 確かめたこと
- `cargo test --workspace --no-fail-fast` 全 pass，`cargo clippy --workspace --all-targets -- -D warnings` clean，`git diff --check` clean。
- `rules.py --base HEAD --renames <作業 dir の renames.txt>`：残る問題は 2 件だけで，どちらも `plan/phase8/reviews/ma1-opus.md` が無いための link（orchestrator が置く）。renames.txt は前回のまま（今回の改名した試験は HEAD で引用されていないので renames は不要）。
- mutation（`maintenance,phase3,phase7 sim02:4,sim03:4`，55 行）：55/55 killed。行：P158〜P160，P168，P170，P175〜P178，P181〜P183，P186，P187，P190，P191，P193〜P198，P589，P965〜P970，P973，P989，P1022〜P1037，M31〜M33，M46，M55，M59，M63，U21。最初の実行は staged ファイルが mode 600 で全行失敗（chmod 644 して再実行）。

## 注意
- 保守的な違いは e が segment の origin から W 未満のとき：Mock は untimed，UHD は timed を渡しうる（MR-34 の Ceiling と KL-32 に書いた）。Session の更新（lead 2 ms）が segment の始まりに当たるとこれに入る。
- red（旧規則で落ちること）は revert して確認していない。mutation の killed が証拠。
- v3/ と .claude/ は mode の検査対象外（触っていない）。
