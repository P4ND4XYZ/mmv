# 初期実装の検証結果

[全体像](index.md) · [実行方法と試験条件](running.md) · [検証計画](verification.md)

## 対象と環境

V-01・V-02向けの初期実装について、自動チェックとWindowsでの起動・終了のスモークテストを実施しました。全検証項目の完了や、GPUIの採用決定を示すものではありません。

| 項目 | 条件 |
|---|---|
| 実施日 | 2026-11-07 |
| ソース | ベース`a4b93eea3ba4b95229cddba7b570768b9ea3e854`に、この初期実装の未コミット変更を適用した作業ツリー |
| OS | Windows 11 Home、`10.0.26300` |
| CPU | AMD Ryzen 7 7700 |
| メモリ | OSから取得した物理メモリ約31.1 GiB |
| GPU | NVIDIA GeForce RTX 5070 Ti / AMD Radeon Graphics（GPUIが使用したデバイスは未特定） |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)`、`stable-x86_64-pc-windows-msvc` |
| Cargo | `1.97.1 (c980f4866 2026-06-30)` |
| CMake | `4.4.2` |
| ライブラリ | GPUI Kit/component/base `0.9.0`、GPUI/HTTPクライアント`0.3.11`。推移的依存関係は`Cargo.lock`参照 |
| ビルド | Debug、`[profile.dev] debug = "limited"`。Releaseは未実施 |
| Node.js / mermaid-cli | `v24.17.0` / `12.0.0`の存在とバージョンのみ確認。図生成は未実施 |

## 実行したチェック

| コマンド・確認 | 結果 |
|---|---|
| `cargo fmt --check` | 成功 |
| `cargo check --locked` | 成功 |
| `cargo test --locked --features ui-tests -j 4` | 10件成功（計算・状態・実ファイル読み込み8件、GPUI結合テスト2件） |
| `cargo clippy --locked --all-targets --features ui-tests -- -D warnings` | 成功 |
| `cargo build --locked -j 4` | 成功 |
| `target\debug\my-markdown-viewer.exe --help` | ウィンドウを開かずに操作概要を表示し、終了コード0 |
| Debug実行ファイルを引数なしで起動 | `GPUI Markdown evaluation`ウィンドウの生成と、本文paintの参考出力を確認 |
| 同ウィンドウを`CloseMainWindow()`で閉じる | 終了コード0。プロセスを残さず終了 |

MSVCのリンク時に、インポートライブラリと`.exp`の作成を知らせる`linker stdout`警告が出ました。ビルド・テストは成功しています。LSP診断も試しましたが、プライマリ言語サーバーから応答が得られず未確認だったため、上表のCargoチェックを判断根拠としています。

### V-01 基本描画

- [入力文書](fixtures/basic.md)を指定せずに起動でき、文書読み込みと対応する解析結果の最初のpaintまで到達しました。
- ローカル画像の解決は、文書相対パス、空白のURLエンコード、絶対パス、`file://`、Web URLを自動テストで確認しました。
- 同梱文書全体の目視確認は未実施です。日本語の字形、表・チェックリスト、コードの色、ローカル・Web画像の実際のロードと描画品質は未確認です。
- シンタックスハイライトの追加設定は`syntect`機能、入力言語はRust・JSONです。独自の解析器やライブラリ改修は使用していません。

### V-02 検索と一致位置への移動

GPUIの実際の`TextView`と検索入力を使ったテストで、次を確認しました。

- `**日本語**`と`[日本語](https://example.com)`の表示テキストには記法・リンク先URLが含まれず、2つの「日本語」を別のUTF-8範囲として取得できる。
- 検索欄の`n`は文字入力となり、本文での`n` / `N`は一致箇所の移動となる。
- 強調APIが一致範囲を受け付ける。80段落の入力で末尾を検索し、表示要求後に実際のリストの論理スクロール位置が先頭より後へ移動する。

強調のピクセル単位の位置、折り返された一致の見た目、日本語IME、GPUでの描画は未確認です。テスト実装中に検出した`TextView::new`の既定値によるスクロール無効化は、要素側へ`scrollable(true)`を設定して修正し、失敗していた結合テストの成功を確認しました。

### V-03〜V-06

未実装・未実施です。リンク座標取得に関する公開API調査上の制約と、次段階で判断する選択肢は[実行方法](running.md#次段階のapi上の制約)に記載しています。

Debugのスモークテストで、標準エラーに次の出力がありました。

```text
first_content_paint_from_main_ms=692.321
```

これは`main`開始からpaintコールバックまでの1回の参考値です。ビルド直後の実行であり、キャッシュを制御していません。プロセス生成を起点とするRelease計測、画像ロード完了、Mermaid生成時間、繰り返し起動の測定ではありません。V-06の結果や起動時間の保証には使用しません。
