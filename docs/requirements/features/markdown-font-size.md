# Markdown文字サイズのLua API

[全体像に戻る](../index.md) / [表示・配色設定](display-settings.md)

## 目的と使い方

Markdownの本文基準サイズを、起動時設定と実行時Lua APIで変更します。単位はWindows表示倍率に追従する論理pxです。標準キー割り当てやプラグインのロードは追加しません。

```lua
local mmv = require("mmv")
mmv.setup({ view = { font_size = 18 } })
mmv.keymap.set("ctrl-=", function() mmv.markdown.adjust_font_size(1) end)
mmv.keymap.set("ctrl--", function() mmv.markdown.adjust_font_size(-1) end)
mmv.keymap.set("ctrl-0", function() mmv.markdown.reset_font_size() end)
```

## 入力と結果

| 関数 | 契約 |
|---|---|
| `mmv.markdown.get_font_size()` | Markdown表示時は適用済み本文基準サイズ、文書なし／PDF表示時はnil |
| `mmv.markdown.set_font_size(size)` | 8〜48（両端を含む）の有限numberを指定 |
| `mmv.markdown.adjust_font_size(delta)` | 有限numberを加算し、結果を8〜48へ収める。負数・0も許可 |
| `mmv.markdown.reset_font_size()` | 起動時の`view.font_size`、未指定なら16へ戻す |

- `setup({view={font_size=...}})`も8〜48の有限numberを受け付けます。整数・小数とも使用できます。
- 標準サイズ16は、既存のGPUI既定文字サイズ1remとcomponent themeのremサイズ16pxから継承していた実際の本文サイズです。
- 引数の個数は表どおり厳密です。不正な型、NaN、無限大、set/setupの範囲外値は副作用前にLuaエラーとなります。数値文字列の変換はしません。options、owner、on_completeはありません。
- 変更関数はイベント内だけで利用でき、受理時はtrue、呼出し時点でMarkdownがなければfalseです。同値、上限・下限、delta=0でも受理します。trueは再描画の完了を意味しません。
- getterは実行時のイベント・render/render_item内で利用できます。init中は全実行時APIを禁止し、render/render_item中は変更を禁止します。起動時設定はinit専用です。

## 寿命・描画・保持

- サイズは文書別ではなくビューアー単位のアプリケーション状態です。文書置換・再読み込み・履歴移動・PDF往復でも保持し、PDFの倍率や描画は変更しません。
- 受理した要求は文書置換をまたいでも順に適用します。相対要求は先行要求の適用結果を使い、getterは未適用の未来値を返しません。
- 本文、見出し、表、インラインコードは既存の相対比率で変わります。独立した等幅サイズを持つfenced codeも従来の本文との比率で変わります。コードの言語ラベルは固定です。
- ツリー、状態行、フローティングUI、画像、Mermaidのサイズは変更しません。グローバルthemeやremサイズは変更しません。
- 文字サイズ変更時は既存のスクロール状態（ブロック位置とブロック内のpxオフセット）を引き継ぎ、先頭へ戻しません。縮小後に有効範囲を超える場合は末尾側の範囲へ収めます。同じ文字が画面内の同じ位置に残ることや、文書全体の絶対px位置の厳密な保持は保証しません。新しい遷移やソース位置指定を上書きしません。
- 完了済みの論理選択、検索語・一致・現在位置と配色を保持し、表示位置を再計算します。modeとfocusは変更しません。
- 設定ファイルへの書き込みや永続化はしません。再起動時は起動時設定へ戻ります。
