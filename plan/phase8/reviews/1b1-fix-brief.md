# 修正の指示書：Phase 8 段階 1b-1（レビュー後）

あなたは実装役です．リポジトリ /home/komatsu/local_work/Ez-SDRv4．前回の指示書 `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b1/brief.md` の「守ること」はすべてそのまま有効です（`cargo fmt`/`rustfmt` 禁止，mutation は `remote_mutate.py` だけ，commit/push/`git add` 禁止，2分を超えるコマンドは nohup で背景，テストを弱めない，版を上げない，新しいファイルは `chmod 644`）．レビューは2本：`~/.cache/ezsdr-fixes/2026-10-17-phase8-1b1/opus-review.md`（O1–O4）と `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b1/sol-review.md`（1–3）．**両方を全文読んでから**始めること．採否：**全部採用**．以下が決定です（レビューの修正案と食い違うところはこちらが優先）．

## 直すこと

1. **`key_decl` を1回の探索に（O1）**：`.find_map(|v| v.keys.iter().find(|d| d.declares(key)))` の1本にする（登録が重なりを拒否するので，当たる宣言は高々1つ）．doc comment の「every exact declaration is consulted first」と SB-2a の「`key_decl` consults the exact declarations before the numbered ones」を消す．mutation 行 P23 を削除し，P22 の `old`/`new` を新しいコードに当て直す（番号付きの宣言を引かない mutant，kill を確かめる）．
2. **Vocabulary の prefix の外の宣言を登録で拒否（O2，Sol 1，major）**：`check_key_declarations` の最初に，`indexed` の有無にかかわらず `decl.key.has_prefix(&v.prefix)` でない宣言を拒否する（文面は key と prefix を名乗る，`MA-34:` で始める）．テスト：完全一致の宣言と番号付きの宣言のそれぞれで拒否；Sol 1 の筋書き（prefix `test` の Vocabulary が `other.item`(1) を持つ → 登録で拒否されるので，その後の prefix `other` の Vocabulary の `other.item0` は受理される）．mutation 行1つ．既存のテストや Vocabulary（radio，sink，テストの語彙，`session.rs` の在庫）で prefix の外を宣言しているものがあれば報告に書く（テストを弱めて通さない）．MA-34 の本文に「登録が prefix の外の宣言を拒否する」を足し，`*Test:*` を足す．
3. **SB-2a の重なりの定義と「綴り」の文（Sol 2）**：等しさの枝を「両方とも番号付きでない位置では segment が等しい」に限定し，3つの場合（どちらも番号付きでない／片方だけ番号付き／両方番号付き）を排他に書く．「key の綴りは宣言が覆う key ではない」は「番号付きの宣言はその綴りを覆わない」に直す（同じ綴りの完全一致の宣言は別にあり得る）．テスト：完全一致 `test.c.x` と番号付き `test.c.x`(1) の併存が登録で受理され，`key_decl(test.c.x)` は完全一致の宣言，`key_decl(test.c0.x)` は番号付きの宣言を返す（kind を変えて区別する）．
4. **固定の segment の負例（Sol 3）**：番号付きの位置が真ん中の宣言（例 `test.c.frequency`(1)）に対し，`test.c0.gain`（固定 segment が違う），`test.c0`・`test.c0.frequency.x`（segment 数が違う）が当たらないことを assert．mutation 行：`declares` の固定 segment の比較 `*s == decl[j]` を `true` にする mutant（kill を確かめる）．
5. **MA-32 は SB-2a を引くだけ（O3）**：MA-32 の拒否の列挙を「a `VocabularyDescriptor` whose declarations MA-34 or SB-2a refuses」の形の引用に置き換える（規則は1か所に書く）．
6. **重なりの枝の mutation 行（O4）**：`(false, true) => false` の行を足す（自分が完全一致・相手が番号付き，の順の重なりで kill）．P35 を「完全一致どうしの重なりだけを消す」mutant に置き換える（例：`a.len() == b.len()` → `a.len() == b.len() && (pa.is_some() || pb.is_some())`．同じ綴りの完全一致2つのケースだけで kill されること）．
7. Changes 行（spec 03，spec 05）をこの修正に合わせる（行は増やさない）．`*Test:*` 行は実際に assert していることだけ．

## mutation

流すのは，この修正で `old` かコードが変わった行と新しい行だけ（P22，P35，P19–P34 のうち対象コードが動いたもの，新しい行）：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance sim02:4,sim03:4 <ids>`（背景で）．全部 killed で，各行がちょうど1回一致すること．

## 確かめること

前回の 1〜5（`cargo test --workspace --no-fail-fast`，host clippy，Docker の `--features uhd` clippy，Python 3.14 suite，`rules.py --base HEAD`）と上の mutation．

## セルフレビュー

前回と同じ形で（新しい読み取り専用レビュー役，最大3回）．チェックリストは前回のものに加え：「O1–O4・Sol 1–3 の各項目が上の決定どおりに直っているか」．

## 報告

`~/.cache/ezsdr-fixes/2026-10-17-phase8-1b1/fix-report.md`．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．項目ごとに直したこと（ファイルと要点），テストとそれが捕まえる間違った実装，mutation 行と kill したテスト，確かめることの結果（数とログ），セルフレビューの各回，未解決．
