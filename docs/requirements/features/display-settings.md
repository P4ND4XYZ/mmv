# 表示・配色設定

[全体像に戻る](../index.md)

読み込み先はWindowsでは`%LOCALAPPDATA%\mmv\init.lua`、Linuxでは`<home>/.config/mmv/init.lua`とする。

## Lua API

[ファイルツリー](file-tree.md)の起動時の表示／非表示は`tree.visible`で指定する。ルート判定マーカーは`tree.root_markers`で指定する。
`require("mmv")`で設定APIを取得する。

```lua
local mmv = require("mmv")

mmv.setup({
  view = {
    position = "center",
    max_width = 960,
    font_size = 16,
  },
  colorscheme = "dark",
  tree = {
    visible = false,
    root_markers = { ".marksman.toml", ".git" },
  },
})

mmv.set_hl("Normal", {
  fg = "#cdd6f4",
  bg = "#1e1e2e",
})

mmv.set_hl("Search", {
  bg = "#665c28",
})

mmv.set_hl("SearchCurrent", {
  bg = "#b57614",
})
```

### `mmv.setup(options)`

表示設定と標準カラースキームを指定する。省略した設定は、以下の初期値を使う。

| 設定 | 値と意味 | 初期値 |
|---|---|---|
| `view.position` | `"center"`は本文領域を横方向に中央配置、`"left"`は左側に配置する | `"center"` |
| `view.max_width` | 本文領域の最大幅。単位は論理pxで、左右の外側余白は含めない。正の有限数を指定する | `960` |
| `view.font_size` | Markdown本文の基準サイズ。8〜48の有限number、単位は論理px。実行時変更は[文字サイズAPI](markdown-font-size.md)を参照 | `16` |
| `colorscheme` | 標準カラースキームの`"dark"`または`"light"`を選ぶ | `"dark"` |
| `tree.visible` | 起動時にツリーを表示するかをbooleanで指定する | `false` |
| `tree.root_markers` | ルート判定に使う直下名の文字列配列。指定時は既定配列を置換する | `{ ".marksman.toml", ".git" }` |

### 通信設定

`mmv.setup({ ipc = { port = 43210 } })`でローカル通信のポートを設定する。`ipc`または`port`の省略時は`43210`を使う。通信は`127.0.0.1`を使用する。

### ルート判定マーカー

- `tree.root_markers`を省略した場合は`{ ".marksman.toml", ".git" }`を使う。指定時は既定値へ追加せず、配列全体を置換する。
- 文書の親から祖先へ探索し、いずれかのマーカーがある最寄りのフォルダーを採用する。
- マーカーはファイル・ディレクトリの両方を許可し、内容は読まない。空配列`{}`では探索せず、文書の親を使う。

### `mmv.set_hl(name, attributes)`

標準カラースキームに対し、用途別の配色グループの属性を上書きする。

- `attributes`には文字色の`fg`と背景色の`bg`を指定できる。ただし`Search`・`SearchCurrent`は`bg`のみ、`Border`は線色を指定する`fg`のみを受け付ける。これらのグループに対応しない属性を指定した場合は設定エラーとする。色は`"#RRGGBB"`形式とする。
- 指定しない属性は、選択した標準カラースキームの値を引き継ぐ。
- 同じグループを複数回指定した場合は、指定された属性ごとに後の値を優先する。後の呼び出しで省略した属性については、それまでの上書きを維持する。
- 文字装飾や配色グループ間の参照を指定する属性は提供しない。

| グループ | 対象 |
|---|---|
| `Normal` | 基本の文字色と背景色 |
| `Border` | UIの境界線 |
| `Search` | 現在選択中ではない検索一致 |
| `SearchCurrent` | 現在選択中の検索一致 |
| `StatusLine` | 下部の状態行 |
| `CommandLine` | 下部の入力行 |
| `ModeNormal` | NORMALのモード欄 |
| `ModeSearch` | SEARCHのモード欄 |
| `ModeCommand` | COMMANDのモード欄 |
| `Heading1`〜`Heading6` | 各レベルの見出し |
| `Link` | 本文のリンク |
| `CodeBlock` | コードブロックの基本色と背景色 |
| `Quote` | 引用 |
| `Panel` | 共通の選択パネル |
| `PanelSelected` | 選択パネルの選択中の候補 |
| `Error` | エラー通知 |
| `Warning` | 警告通知 |

### 配色の対象範囲

- 本文・見出し・リンク・引用・コード背景・検索一致・下部UI・選択パネル・通知を対象とする。
- コードのシンタックスハイライトは、選択した標準カラースキームに合わせる。トークン単位のLua設定は対象外とする。
- `Link.bg`はMarkdown記法のリンクの表示テキストに適用する。HTMLブロック内のリンクは対象外とし、従来の文字色とクリック操作は維持する。リンク内の画像自体には背景を描かない。
- Mermaid図内の配色と画像の色は変更しない。

## 設定に失敗した場合

- ファイルが存在しない場合はエラーにせず、標準設定を使う。
- ファイルの読み込み、Luaの実行、設定値の検証に失敗した場合は、設定を部分適用しない。表示・配色・ツリー・通信（port 43210）を含むすべての設定とLua登録を組み込み標準へ一括で戻す。CLIは標準値で接続・起動を試せるものとし、GUIは標準設定で起動して閲覧を継続する。`init.lua`は書き換えない。
- 未知の設定名・配色グループ名・属性名、未対応の文字列、不正な型・色・数値は設定エラーとする。
- 設定エラーは下部入力行に英語の`Unable to load configuration`を表示し、設定ファイルのパスと原因を既存の`viewer.log`へ記録する。保存先と書き込み失敗時の扱いは[エラーログ](logging.md)に従う。
