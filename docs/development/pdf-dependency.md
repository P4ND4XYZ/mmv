# PDF依存の取得と検証

## 現在の範囲

固定Git依存と薄いadapterを、runtimeの実解析・prepared配送・取消registry、排他的なMarkdown/PDF本文ビューへ接続しています。バイト列の読取だけでは成功せず、実解析と全ページ寸法の検証後に共通の文書採用・CLI応答を行います。同一PDFの通常openは再読込・再解析・view再生成をせず、失敗や古い要求では現在の表示を保持します。PDF上の検索は未対応通知に接続し、Search中の入力・確定済み検索語と既存IMEガードを維持します。

本文フォーカスのNormalでスクロール・拡大縮小・冪等な幅合わせを実adapterへ接続し、ツリーの通常`.md`／`.pdf`選択、Markdown内リンク、CLIの共通open・履歴を実parser・headless view・隔離Named Pipeで自動検証する構成です。Search・Command・選択パネル・ツリー・IMEの入力分離も試験対象としますが、native GPU・実shutdown・IME実機・代表PDFの性能確認まで完了した意味ではありません。機能上の正本は[PDF表示の要件](../requirements/features/pdf-viewing.md)と[PDF接続契約](../design/components/pdf-viewing-contract.md)です。

## PDFを開く・操作する

Windowsのリポジトリ直下でのビルド・起動例です。実機確認前に[チェックリストの対象版・singletonの注意](../testing/pdf-manual-checklist.md#実施前の安全確認)を確認してください。

```powershell
cargo build --locked --bin mmv --target-dir target
.\target\debug\mmv.exe open "C:\pdf-check\sample.pdf"
# 省略形（同じファイルを開く）
.\target\debug\mmv.exe "C:\pdf-check\sample.pdf"
```

- ツリーでは通常の`.pdf`／`.PDF`を選び、Enterまたは`l`で開きます。表示切替とフォーカス移動は[ファイルツリー](../requirements/features/file-tree.md#入力とpane移動)を参照してください。
- MarkdownのローカルPDFリンクはクリック、またはNormalの`f`でリンク選択パネルを開いて確定します。相対パスはリンク元の文書基準です。[リンクと履歴](../requirements/features/links-history.md#リンク)に従い、同じ本文領域へ切り替わります。
- 本文フォーカスのNormalでは`j`／`k`（40論理px）、`Ctrl+d`／`Ctrl+u`（半viewport）、`Ctrl+f`／`Ctrl+b`（全viewport）、`gg`／`G`、ホイールで移動します。PDFページ単位のジャンプではありません。
- 初期表示と`=`は幅合わせです。幅合わせ中はresize・ツリー表示による本文幅変更に追従します。`+`／`-`は×1.1／÷1.1で幅合わせを解除し、中央付近のPDF位置を保ちます。入力条件・詳細は[スクロールとズーム](../requirements/features/pdf-viewing.md#スクロールとズーム)が正本です。

### 初期版で行わないこと

PDF検索・自動更新・`F5`再読み込み・`--line`は対象外です。Normalの`/`・`n`・`N`は検索せず既存INPUT行へ通知し、PDFへの`--line`は同一openでも未対応エラーです。ページ指定にはなりません。Search中に外部CLIでPDFへ切り替えた場合の入力・IME・検索語の保持は[初期版の検索規則](../requirements/features/pdf-viewing.md#初期版の検索再読み込み履歴)と[本文検索](../requirements/features/search.md)を参照してください。

同一PDFの通常openは再読み込みせず、位置・倍率・履歴を保ちます。一方、`Ctrl+o`／`Ctrl+i`やCLIの`back`／`forward`によるPDFへの履歴復帰は先頭・幅合わせです。Markdownの検索・更新・位置復元は従来どおりです。[外部操作](../requirements/features/external-control.md)も参照してください。

## 固定した取得元

`Cargo.toml`では`gpui-pdf`だけを次のGit依存として指定し、`default-features = false`にしています。

- fork: <https://github.com/P4ND4XYZ/zorite.git>
- revision: `88fcd079a5724a170d063a4b114c6427edc0c5df`（完全なcommit SHA）

Cargoがリポジトリを取得し、workspace memberの`gpui-pdf`を選択します。workspaceのmanifest継承を解決するためのリポジトリ取得と、全memberのビルドは別です。このアプリの依存を検証する際、Zorite本体やworkspace全体をビルドする必要はありません。

隣接するforkのcheckout、絶対ローカルパス、submodule、crateのコピーは不要です。PDF依存のためにPerlを導入する必要もありません。既存のWindows向けRust/MSVC・GPUIビルド環境は引き続き必要です。

既存の`gpui-kit = 0.9.0`と`gpui-pre = 0.3.11`の構成を維持し、`gpui-base`のpatchは引き続き`https://github.com/P4ND4XYZ/gpui-kit.git`の`1a2539f67f4dbceecb1885c02abd272dbda06e33`に固定します。

## Cargo.lockと通常の検証

`Cargo.toml`とCargoが生成する`Cargo.lock`を一緒に管理します。初回はGitリポジトリと必要なcrateの取得にネットワーク接続が必要です。完全SHAの固定だけで、取得元の将来的な可用性まで保証するものではありません。取得済みのGit・crateキャッシュは再利用できます。

通常の確認はアプリのリポジトリ直下で実行します。

```powershell
cargo metadata --locked --format-version 1
cargo tree --locked
cargo check --locked --bin mmv -j2 --target-dir target
cargo fmt --check
```

必要な依存をすべて取得済みなら、`cargo metadata`と`cargo tree`に`--offline`を追加して固定した依存graphを確認できます。`--locked`でlockfileの更新が必要と判定された場合、無条件に制約を外したりlockfileを手編集したりせず、依存変更の理由を確認してください。

revision更新時はforkの変更をレビューし、新しい完全SHAへ固定したうえでCargoにlockfileを生成させます。既存packageのversion・source・feature、GPUIのsource identityと重複、追加される推移依存を比較し、実アプリの`--locked` checkをやり直します。無関係なupgradeやZorite本体の依存を取り込まないことを確認します。

## ライセンスと未検証事項

利用する`gpui-pdf` crateはMIT、ZoriteのrootアプリはGPL-3.0-or-laterとして区別します。GPL本体のコードをコピー・流用する構成ではありません。取得元の著作権・ライセンス表示は保持します。推移依存を含むライセンス監査が完了したとは扱いません。

利用者が行う[PDF実機チェックリスト](../testing/pdf-manual-checklist.md)は全項目未実施から始めます。T3/T4の実装は完了していますが、prototypeへの統合はLua UIの競合解消待ちです。統合後の対象版を特定してから実機確認を行ってください。

依存の解決・ビルド確認は、実機でのPDF動作確認の代わりにはなりません。実装接続後のnative GPU表示と資源解放、置換・終了時のshutdown、Search/CommandとIME、日本語・画像多数・数百ページの代表PDFによる性能・メモリ確認は別の検証です。headless試験は実機のGPU・OS終了・IME動作や代表PDFの測定を代替しないため、それらの成功や測定結果を主張しません。自動検証の失敗を、この手動確認へ暗黙に付け替えるものでもありません。
