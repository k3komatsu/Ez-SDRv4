# 修正の指示書：Phase 8 段階 1b-2b（レビュー後）

あなたは実装役です．リポジトリ /home/komatsu/local_work/Ez-SDRv4．前回の指示書 `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/brief.md` の「守ること」はすべてそのまま有効です（`cargo fmt`/`rustfmt` 禁止，mutation は `remote_mutate.py` だけ，commit/push/`git add` 禁止，2分を超えるコマンドは nohup で背景・終わりは `kill -0 <pid>`，テストを弱めない，版を上げない，新しいファイルは `chmod 644`，mutant の切り分けもリポジトリの外のコピーで）．レビューは2本：`~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/opus-review.md`（O1–O2）と `~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/sol-review.md`（1–4）．**両方を全文読んでから**始めること．採否：**全部採用**．以下が決定です（レビューの修正案と食い違うところはこちらが優先）．

## 問題の根（Sol 1，Sol 2，major）

移動先の ready（移動元が channel を離す instant）を記帳のときに1回だけ計算して移動先の item に焼き込んでいる．移動元の計画はその後の記帳（先行の `cold` 変更による開始待ちの segment，後から来た `start_rx`，拒否，fault）で変わりうるので，焼き込んだ値は古くなり，移動先が移動元より先に channel を取る計画ができる．2本の line を別々の値として持つ限り直らない．設計 33 D15 は「instance の全 stream の line は1つの lock の下」と決めているので，**instance の全 line を1つの型にまとめ，移動の依存をその型が保つ**．

## 直すこと

1. **instance の line をまとめる型（Sol 1，Sol 2）**：`timeline` に，1つの instance の全 stream の line を持つ型を置く（例：`Lines`．stream は方向と番号で引く，例 `(Direction, u32)`）．操作はすべてこの型を通す：`book(stream, item)`，`refuse(stream, seq)`，`prune(before)`，`book_move(from, to, channel, at, now, seq, ready)`，`refuse_move(seq)`，`config_at(stream, e)`，各 line の読み出し．移動は `(seq, from, to, 渡された ready)` として型の中に記録する．
2. **移動元の release を計画から読む**：line の replay で，`Cold` の item が running の segment（開始待ちで origin が item の instant より後の segment を含む：replay がそれを origin で cut する）を cut したとき，その cut の instant を item の `seq` とともに記録し，line が `seq` の cut の instant を返せるようにする（例：`Line::cut_by(seq) -> Option<i64>`．running の segment が無ければ None）．移動先の item の ready は「渡された ready と，移動元の line の今の計画での `cut_by(seq)` の遅い方」．
3. **依存を保つ**：`Lines` のどの操作（記帳，拒否，prune，移動，移動の拒否）の後でも，記録した全移動について移動先の item の ready を今の計画から計算し直し，変われば移動先の line を計画し直す．移動先の計画の変化が別の移動の移動元の cut を変えうるので，変化が無くなるまで繰り返す（回数の上限は移動の数＋1 とし，超えたら内部の誤りとして `TimeError` か panic．どちらにしたかを報告に書く）．操作が失敗したらすべての line を元のまま（今の clone してから確定する形）．
4. **ありえない移動を拒否（O1，Sol 3）**：`book_move` は，`channel` が 64 以上，移動元の `config_at(e)` に無い，移動先の `config_at(e)` に既にある，`from == to`，のどれかなら記帳せず，全 line を元のままにして拒否する．専用の error 型（例：`MoveError { Time(TimeError), NoSuchChannel, NotInSource, AlreadyInDestination, SameStream }`）で返す．RM-34 に書く．
5. **`config_at` を公開（O2）**：`Lines::config_at(stream, e)`（prune 済みの状態を含む，その instant に効いている config）を公開し，RM-34 に「Provider は後の `cold` 変更の config をこれから作る」と書く．
6. **RM-13 を layout の description に合わせる（Sol 4）**：layout の description では，`TxBurst` の波形の channel 数は宛先の送信 stream に入っている channel の数で，波形の channel `i` はその stream の論理 channel 番号の小さい順の `i` 番目（受信 block の channel の順と同じ，設計 33 §3「channel の番号の小さい順に block の channel になる」）．RM-3 の受信側にも layout の description の同じ対応が書かれているか確かめ，無ければ足す．
7. 前回の `book_move`／`refuse_move` の自由関数は，`Lines` の操作に置き換える（2本の `&mut Line` を受ける形は残さない：依存を保てないため）．Mock・UHD は今まで通り `Line` を直接使う（1方向1 stream で移動しない）ので，`Line` の公開の操作はそのまま残す．

## テスト

- Sol 1 の筋書き：移動元に先行の `cold` 変更があり次の segment の origin が移動の instant より後，その同じ instant に空の移動先への移動 → 移動先の segment の origin が移動元の cut（その origin）＋ lead 以後．
- Sol 2 の筋書き：停止中の移動元（受信，off）から空の移動先への移動を先に記帳し，その後に移動元へ移動より前の `start_rx` を記帳 → 移動元は移動まで走り，移動先の segment は**新しい**移動元の cut ＋ lead 以後に動く（記帳の後の計画し直しで）．
- 連鎖：A→B の移動の後に B→C の移動，A の計画を変える記帳で C の開始も正しく動く．
- 依存の保持の一般形：移動を含む生成列（VH-8 の生成器）で，全移動について「移動先の新しい segment の origin ≥ 移動元の `cut_by(seq)` ＋ lead」が成り立つ（line ごとの不変条件に加えて，line 間の不変条件として `check` に足す）．
- O1/Sol 3：4つの拒否それぞれで error が返り，全 line が元のまま（計画も items も等しい）．
- O2：移動の後と prune の後の `config_at`．
- 拒否・失敗で全 line が元のまま，は今のテストを `Lines` に移して残す．
- mutation の行：移動先の ready を計画し直さない（記帳時の値のまま），`cut_by` が開始待ちの segment を無視する，依存の繰り返しを1回で止める，各拒否の検査を外す，`config_at` が prune 済みの状態を無視する．前回の P81〜P92 で対象のコードが変わる行は当て直す（kill を確かめる）．

## 文書

spec 07 の RM-34 を新しい形に（instance の line をまとめる型，移動の記録，release は移動元の今の計画の cut，どの操作の後も依存を保つ，ありえない移動の拒否，`config_at`）．RM-13（と必要なら RM-3）の layout の対応．RM-26 の「one implementation」の記述を合わせる．DA-1 形式（規範だけ，段階を書かない），`*Test:*` は実際に assert していることだけ．Changes 行は今の 1b-2b の行を書き換える（行を増やさない）．

## mutation

流すのは，この修正で `old` かコードが変わった行と新しい行（列挙して報告）：`REMOTE_SSH="ssh -o BatchMode=yes" python3 plan/maintenance/tools/remote/remote_mutate.py maintenance,phase7 sim02:4,sim03:4 <ids>`（背景で）．全部 killed で，各行がちょうど1回一致すること．

## 確かめること

前回の 1〜5（`cargo test --workspace --no-fail-fast`，host clippy，Docker の `--features uhd` clippy，Python 3.14 suite，`rules.py --base HEAD`）と上の mutation．

## セルフレビュー

前回と同じ形で（新しい読み取り専用レビュー役，最大3回）．チェックリストは前回のものに加え：「O1–O2・Sol 1–4 の各項目が上の決定どおりに直っているか．line 間の依存がどの操作の後でも保たれるか（Sol 1・2 の筋書きのほかに自分で反例を探す）」．

## 報告

`~/.cache/ezsdr-fixes/2026-10-17-phase8-1b2b/fix-report.md`．最初の行は `STATUS: done` か `STATUS: blocked: <理由>`．項目ごとに直したこと（ファイルと要点），テストとそれが捕まえる間違った実装，mutation 行と kill したテスト，確かめることの結果（数とログ），セルフレビューの各回，未解決．
