# my-markdown-viewer 要求・要件

## 対象範囲と制約

- 読み取り専用のviewer。

## 機能一覧

| 機能 | 利用上の役割 | 要件 |
|---|---|---|
| ファイル表示・更新 | 文書を開き、編集結果を確認する | [ファイル表示と更新](features/file-viewing.md) |
| ファイルツリー | ファイルのツリーをを表示する | [ファイルツリー](features/file-tree.md) |
| Markdown描画 | 本文・表・コード・画像を読む | [Markdown描画](features/markdown-rendering.md) |
| PDF表示 | 同じ本文領域でPDFを読み、スクロール・ズームする | [PDF表示](features/pdf-viewing.md)、[PDFズームのLua API](features/pdf-zoom.md) |
| 画面・操作モード | 閲覧・検索・コマンド入力を切り替える | [画面と操作モード](features/viewer-ui.md) |
| 表示・配色設定 | Lua設定で本文領域の配置・最大幅と配色、ツリー初期表示と判定マーカーを指定する | [表示・配色設定](features/display-settings.md) |
| LuaフローティングUI | キーから一時的なUIを開き、一覧・入力・ソース・ボタンを合成する。同梱Luaでも既存リンク選択を構成する | [利用と範囲](features/lua-floating-ui.md)、[公開API](features/lua-api.md) |
| キーボード操作 | キーボード中心に本文を読み進める | [スクロールと先頭・末尾への移動](features/navigation.md) |
| 本文検索 | 必要な記述へ移動する | [本文検索](features/search.md) |
| リンク・履歴 | マウスクリックや絞り込み付きリンク選択で関連文書・見出し・Webページを開き、履歴で閲覧位置を行き来する | [リンクと履歴](features/links-history.md) |
| CLI・外部操作 | ファイルを開き、エディター側から表示を操作する | [外部操作](features/external-control.md) |
| Mermaid描画 | 文書内の図を読む | [Mermaid図](features/mermaid.md) |
| エラーログ | 図の生成失敗や設定エラーの原因を確認する | [エラーログ](features/logging.md) |
