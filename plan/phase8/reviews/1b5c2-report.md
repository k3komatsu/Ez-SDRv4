# 1b-5c-2：実装役（Luna）の報告

STATUS: done

## 完了内容

Phase 8 stage 1b-5c-2 を完了。旧 current-key description と selector `keys` を削除し、実装・テスト・spec・Vision・schema を layout-only に統一した。Mock/UHD/server/Python client の動作、`radio.full_duplex` など維持 key は変更していない。`python/ezsdr` は無変更。

変更前 HEAD は `b3e6ae2`、tree は clean。現在は 36 files、全体 `+499/-1796`。crate 本体 `+48/-935`、test files `+170/-512`。変更ファイルの mode はすべて 644。

### 1. 削除した本体

| ファイル | 削除／変更 | 残したもの |
|---|---:|---|
| `crates/ezsdr-radio/src/device.rs` | 354行削除、3行追加 | `Grid`、rate-set 比較など layout が使う共通処理 |
| `crates/ezsdr-radio/src/x3xx.rs` | 236行削除、5行追加 | layout 用 board/slot/front-end 表、profile terms と共有 helper |
| `crates/ezsdr-radio/src/lib.rs` | 293行削除、9行追加 | 14個の維持 key、layout keys、共有登録処理 |
| Mock/UHD provider source | `channel_keys`／selector `keys` 分岐削除 | layout description だけを使う実装 |

`DeviceDescription`、旧専用 builder/coercion、`Selection`、`x3xx::from_selector`、`x3xx::description`、置換済み key 宣言と `channel_keys` を除去。維持 key は `radio.full_duplex`、`radio.hardware_time`、`radio.tx.repeat_*`、`radio.rx.block_len`、`radio.timing.*`、`radio.perf.*`。selector の `keys` は Mock/UHD とも未知 key として拒否する。

### 2. テストの移動・削除・追加

| 旧テスト／ケース | 扱い | 残った確認 |
|---|---|---|
| `rm_26_the_description_builds_the_tree` | 旧 `DeviceDescription` test を削除 | `rm_29_the_layout_description_builds_the_stream_tree`、`rm_29_the_ideal_layout_builds_its_tree_and_defaults`、tree を両 Provider で比べる `d3_*` acceptance tests |
| `rm_26_the_description_is_the_one_profile_both_providers_name` | 旧 profile description test を削除 | `mr_03_profile_values_reach_the_capabilities_and_the_envelope_section`、`ur_09_profile_values_reach_the_capabilities_and_the_envelope_section`、CBX/OBX tests と両 Provider の `d3_*` acceptance tests |
| `rm_26_the_decimation_set_is_the_ddc_s` | 同一テストを `x3xx_the_decimation_set_is_the_ddc_s` に改名 | 同名の全639 decimation/rate/coercion checks。mutation filters も改名 |
| `rm_26_the_layout_parser_checks_the_shared_selector` | layout parser test へ移動・改名 | `rm_28_the_layout_analysis_cases` が混在 layout と各 refusal、順序・重複を確認 |
| `rm_26_coerce_cases` | layout coerce test へ移動・改名 | `rm_30_coerce_cases` と Mock/UHD の `mr_06_coerce_cases`、`ur_11_coerce_is_the_descriptions` |
| `rm_26_the_envelope_carries_each_directions_start_lead` | legacy description test を削除 | Mock `mr_03_profile_values...`、UHD `ur_09_profile_values...` の capability/Manifest assertions と `rm_20_schema_freeze` |
| `rm_38_the_keys_selector_is_read_once_for_both_providers` | 共有 parser 自体が廃止されたため削除 | 個別の拒否確認 `mr_38_keys_is_an_unknown_selector_key` と `ur_05_keys_is_an_unknown_selector_key` |
| Mock `mr_38_a_layout_instance_uses_one_key_set` | selector-mode test を unknown-key test に置換 | `mr_38_keys_is_an_unknown_selector_key` は `keys: "channel"` を拒否 |
| UHD `ur_05_the_keys_value_picks_one_key_set` と `ur_05_every_other_keys_value_is_refused` | 二つを廃止し、値に関わらず未知 key として拒否する一件に統合 | `ur_05_keys_is_an_unknown_selector_key` と `ur_05_from_binding_refusals` |
| `rm_33_rm_19_per_channel_cases` の旧 current-key RF case | legacy-key case を削除 | 同テストの layout channel cases と `rm_19_rf_envelope_cases` |

ほかの Mock/UHD/acceptance/server/Python fixtures から selector `keys` を除去。空 configuration になっていた TX-disabled RF-envelope case は、設定を持つ `stream = -1` channel に変更し、空照合で pass しない assert を加えた。新しい `mr_30_untimed_receive_tune_precedes_transmit_at_one_origin` は共有 origin の loopback phase draws で RX→TX order を検証する。

### 3. Mutation rows

- source を削除したため除去した行：maintenance `P62`、`P122`、`P283`、phase7 `V02`。各 `old` 文字列を現 `crates/` に照合し、すべて0件。
- layout へ当て直した主な行：`J270/J271` → `rm_29` tree test、`P04–P09` → `rm_28` parser test、`P15/P17/P18` → layout/acceptance tests、`P138` と `V01` → `mr_06_coerce_cases`、`V03` → `rm_29` tree test。改名した decimation filter と `P281` の unknown-key filter も更新。
- 追加：`P192` は RX/TX tune-order mutant を新 MR-30 test に当てる。`P333` の mutant を削除済み `keys::CONFIGURATION` 参照から `if false {` に変更。
- 削除した型・関数・旧 test 名を mutation `new`/`filter` に grep した結果は0件。残る全 row の source 一致は `rules.py` が0 problem。
- stage 対象20行は最終20/20 killed（V01 は `mr_06_coerce_cases` に付け替え後 killed）。追加の P333 は1/1 killed。
- 全 mutation gate：1257/1258 passed。唯一の survivor は既知の `phase7/G09`。各 list は maintenance 888/888、phase3 83/83、phase4 34/34、phase5 29/29、phase6 63/63、phase7 160/161。

### 4. Design / Vision / schema

- `design/03-spec-and-binding.md`、`07-radio-model.md`、`09-mock-radio.md`、`18-uhd-radio.md` を layout-only に更新。RM-26 は withdrawn stub とし、timeline rule を RM-25 に置く。selector `keys` は UR-5 の未知 key として拒否し、RM-9 は channel ごとの phase behavior のみを規範化。
- Vision §8 の例を layout keys に更新し、§25 の「coherence は channels を所有する instance が宣言し、instances 間で合成しない」を復元。Vision 03/05 と index revision history も更新。
- `crates/ezsdr-kernel/src/spec.rs` の例を layout key に更新し、`schemas/experiment_spec.v1.json` を再生成。schema version は不変。

### 5. 検査結果

- `git diff --check`：pass。
- `rules.py --base HEAD --renames ...`：0 problems。
- `timeout 300 cargo test --workspace --no-fail-fast`：pass。
- `timeout 300 cargo clippy --workspace --all-targets -- -D warnings`：pass。
- UHD Docker clippy：pass。`cargo test -p ezsdr-radio-uhd --features uhd --lib --test uhd_api`：250 tests pass（245 + 5）。
- Kernel `schema_freeze`：5/5 pass。
- Python 3.14 unittest：56 tests pass。
- 全変更ファイル mode 644。commit/push/stage は行っていない。`cargo fmt`/`rustfmt` も未実行。

### 6. Self-review

DeepSeek の初回レビューは P1 なし。指示どおり再レビューなし。

- P2 5件はすべて修正：UR-5 の `keys` 記述、RM-9 の phase-validity claim、RM-2 が引用される TX port reservation、空 configuration の RF test、P333 mutant。
- P3 はすべて修正：RM-26 reason prefixes と stub、spec 18 §6 の MR-38→UR-5、spec 07 §6 の timeline 3 rows、Vision §8 の cross-instance coherence 文、protocol/Python の余分な空白。

### 7. 行数と残り

`crates/` の非 test source は `+48/-935`、test files は `+170/-512`。全体は `+499/-1796`（36 files）。Python client package は未変更。

残りのリスク：実機の ignored hardware tests / Phase 8 bench はこの作業では実行していない。全 mutation gate の唯一の未 kill は記録済み survivor `phase7/G09`。その他に未解決の指摘はない。

## 修正の回 1

1. **Sol 1 — RM-30 coercion coverage** (`crates/ezsdr-radio/tests/channels.rs:516–517, 555–575`): added exact assertions for 19.5 Msps → 20 Msps, gain 31.3 dB → 31.5 dB, and `Set(["LNA", "TX/RX"])` → `TX/RX`; also added the absent TX `c2` case alongside the existing absent RX `c3` case. HEAD's remaining coercion cases are covered as follows: the 30.25 dB tie is covered by `mr_06_coerce_cases`' 20.25 dB tie (same lower-tie rule); 7 GHz refusal and TX/RX throughput refusals by `mr_06_coerce_cases`; fixed two-channel capacity is replaced by channel-absent refusals under the declared layout; `Present` defaults and unknown old keys are already asserted in `rm_30_coerce_cases`; whole-hertz rejection is covered by the ideal profile's fractional-rate refusal in `mr_06_coerce_cases`.
2. **Sol 2 — Vision §8** (`design/vision/02-spec-binding-and-compilation.md:43`, `Ez-SDR_v4_ARCHITECTURE_VISION.md:166`): restored the coherent sensing-array intent and stated that all channels must be successfully timed-tuned for the segment, whose achieved basis/validity the Provider records in its Manifest. The text also distinguishes requested capability from achieved coherence; §25's instance ownership rule remains intact.
3. **Sol 3 — numbered keys** (`design/07-radio-model.md:93`; `design/vision/05-coherence-calibration-and-runtime-semantics.md:110`): frequency/gain step and path-delay descriptions now use only channel-numbered keys. The design/Vision grep found remaining old spellings only in historical Changes rows. Updated current test descriptions in `design/09-mock-radio.md:376` and `design/18-uhd-radio.md:404–405` to say no direction-wide configuration/coherence capability.
4. **Sol 4 — pruning contract** (`design/07-radio-model.md:250,252`): restored the boundary condition (no later booking precedes it, no later refusal reaches pruned items), folding into starting state, and unchanged-plan guarantee; added `rm_26_a_pruned_line_plans_as_one_that_keeps_every_item`. Tree/default/coercion/refusal rules formerly under RM-26 remain in RM-2/RM-4/RM-5/RM-8 and RM-29/RM-30. Updated current spec 09 references at `design/09-mock-radio.md:239,333` to RM-25.
5. **O1 — P333 label** (`plan/maintenance/tools/mutations.json:8542`): renamed it to describe the mutant's actual effect, dropping all layout configuration keys from `prepare`'s effective configuration.
6. **O2 — duplicated rule reference** (`crates/ezsdr-mock-radio/src/lib.rs:1823`): fixed the comment to `RM-25, RM-27`; grep for `RM-25, RM-25` in crate Rust sources returned no matches.

**Repair-round checks:** `timeout 300 cargo test -p ezsdr-radio --test channels` passed (8/8); `timeout 300 cargo clippy --workspace --all-targets -- -D warnings` passed; `rules.py --base HEAD --renames ...` reported 0 problems; `git diff --check` passed. Remote mutation rows P47–P52 and new P549–P551 passed 9/9 on sim02/sim03. The full mutation lists were not run in this repair round; the brief leaves that gate to the orchestrator.
