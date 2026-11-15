# PDF依存の取得と検証

## 現在の範囲

固定Git依存に加え、実parserとhost-controlled viewを包む薄いadapterを追加しています。ただし、runtimeのprepared配送・取消registry、本文のMarkdown/PDF切り替え、検索・キー操作への接続は未実装です。productionのPDF採用はまだ有効にしておらず、アプリでPDFを開けるようになったことや全体の動作検証完了を意味しません。機能上の正本は[PDF表示の要件](../requirements/features/pdf-viewing.md)と[PDF接続契約](../design/components/pdf-viewing-contract.md)です。

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

依存の解決・ビルド確認は、実機でのPDF動作確認の代わりにはなりません。実装接続後のnative GPU表示と資源解放、置換・終了時のshutdown、Search/CommandとIME、日本語・画像多数・数百ページの代表PDFによる性能・メモリ確認は別の検証です。この段階では、それらの成功や測定結果を主張しません。自動検証の失敗を、この手動確認へ暗黙に付け替えるものでもありません。
