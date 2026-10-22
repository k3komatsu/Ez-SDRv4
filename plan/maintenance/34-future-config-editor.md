# 将来構想：設定を見て直せる GUI エディタ

2026-10-22 に owner が出したアイデアの覚え書き．詳細設計はまだしない．着手は v4.0 の凍結がほぼ済んでから．

## 目的

Ez-SDR は AI に SDR を操作させるための道具で，ExperimentSpec と BindingProfile は AI にとって厳密な形を優先している．人間が手で書くには難しい．文書の形は変えずに，人間がそれを

- 見る（設定の可視化），
- 少し直す（AI が書いた設定の微修正），
- 一から作る

ための GUI を作る．対象は外部の利用者まで含む．配り方は単一の HTML ファイルか，localhost で起動するもので足りる．

## 今の時点での方針（案）

- **規則を二重に書かない**．GUI は検査の規則を持たない．フォームの形は schema（`schemas/experiment_spec.v1.json`，`binding_profile.v1.json`）と Vocabulary の宣言（`KeyDecl`）から作り，正しいかどうかは Rust の検査に聞く．
- **単一の HTML に Rust の検査を WASM で埋め込む**のが第一候補．server は stdio の framing で 1 つの client と話すので，ブラウザから直接は呼べない．wasm32 で検査の道が build できるかは未確認（INFERRED）．
- **画面に要るもの**：layout の図（device → board・slot → front end → channel → stream），人間の単位（MHz，dB）での表示と編集，Spec と Binding を別のタブ（§8，§10 の分離），AI の版と人間の版の差分，検査結果を key の位置に表示．
- **守ること**：何も変えずに読んで保存すれば意味が変わらない（round-trip），知らない field を消さない，対応しない version は開かない（§65 #39）．

## 凍結の前に確かめること

- `KeyDecl` は説明文・単位・範囲を持たない．単位は key 名の接尾辞，範囲は Provider の envelope と profile から取れる．説明文は Kernel の型に足さず，Vocabulary の側に置く見込み．凍結の時に「GUI のために Kernel の型を変える必要がない」ことだけ確かめる．
- 前段として役に立つもの（GUI が無くても使える）：Vocabulary の宣言を JSON で書き出すこと，Session を開かずに設定を検査する入口．
