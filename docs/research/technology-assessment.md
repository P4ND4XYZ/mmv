# 技術方針と採用前の検証

[要求・要件](../requirements/index.md)

## この文書について

会話で合意した実装上の方針と、公開資料を用いた予備調査を記録します。機能要件そのものや、実装に進める詳細設計を定める文書ではありません。

資料・公開ソースの調査のみを実施しており、このプロジェクトでのビルド、実機動作、起動時間の計測は未実施です。以下の調査情報は取得時点の内容であり、採用時には利用するリリースとAPIを再確認します。

## 合意した技術方針

### 外部操作

- CLIを外部アプリからの共通の入口にする。
- CLIから起動済みビューアーへの通信には、WindowsのNamed Pipeを使う。
- 単一起動を前提とし、操作対象のインスタンスを利用者が選択する仕組みは設けない。
- 未起動時のスクロールなどは、呼び出されたCLI側で接続失敗を検知し、失敗を返す。未起動のビューアー自身に検知させる方式ではない。

コマンド構文、通信プロトコル、接続先の識別方法などは、この合意では定めていません。

### Mermaid

mermaid-cliとキャッシュを組み合わせる方式を採用候補として検討しています。事前インストールを許容すること、非同期生成、キャッシュ再利用、mermaid-cliがない場合を含む失敗時の表示・ログ方針は合意済みです。

利用するバージョンや呼び出し方法などは、実装設計で扱います。

## GPUI / gpui-componentの予備調査

上流プロジェクトはGPUI Kitという名称になっており、その構成要素にgpui-componentとgpui-baseがあります。TextViewの本体はgpui-baseにあり、gpui-component側には互換APIとテーマ連携があります。

| 項目 | 資料で確認したこと | 採用判断への影響 |
|---|---|---|
| Windows | 対応。開発にはMSVC版Rust、Visual StudioのC++開発ツール、Windows SDKが必要 | 基盤候補にできるが、このプロジェクトでの動作検証は未実施 |
| Markdown | TextViewが見出し、リスト、表、コードブロックなどを描画 | 必要な表示要素をサンプルで確認する |
| シンタックスハイライト | 構成に応じた機能・拡張手段あり | 有効化する構成と言語を選ぶ必要がある |
| 画像 | Web画像を表示可能。相対パスなどのローカル画像はアプリ側で読み込み方針を実装可能 | 文書を基準とするローカル画像の解決を検証する |
| 検索 | 表示テキストの取得、範囲の強調、範囲へのスクロールAPIあり | 検索操作自体はアプリ側で実装する |
| リンク | アプリ側でクリック処理を差し替え可能 | ファイル切り替え、見出し移動、履歴は別途実装する |
| Vimium風のリンク選択 | 完成済み機能としては確認していない | リンク位置の取得とラベル表示の実現性を検証する |
| 閲覧位置の保持 | 合意した動作を満たすかは未確認 | 文書の前方への挿入や図の非同期表示で検証する |
| 起動時間 | 資料だけでは判断できない | WindowsのReleaseビルドで測定する |

GPUIは開発中で破壊的変更があり得ると案内されています。GPUI Kit側も対応するGPUIのスナップショットを揃えているため、採用時には互換性のある依存関係を固定して確認します。

GPUIおよびGPUI KitのソフトウェアのライセンスはApache-2.0です。配布に必要な依存ライセンスの確認全体を完了したものではありません。

### Mermaidとの組み合わせ

調査時点では、標準機能として利用できるMermaid描画は確認できませんでした。関連PR #3685は、GitHub APIの確認時点で未マージのままクローズされています。このPRの機能を、利用可能な標準機能として扱いません。

mermaid-cliは、MermaidソースからSVGやPNGなどを生成できます。GPUI側のMarkdown表示と図の生成結果を組み合わせる方法は、採用前の検証対象です。

Rust製のmermaid-rs-rendererも候補として調べました。ブラウザーを必要としない一方、公式Mermaidと出力が一致するとは限らない旨が案内されています。独自レンダラーの採用を決定したわけではありません。

## 採用判断の状態

- CLIとNamed Pipeによる外部操作：方針合意済み。
- GPUI / gpui-component：候補。採用前の検証が必要。
- Mermaid描画：mermaid-cliを用いる方式を候補として、合意済みのキャッシュ・失敗時要件を満たせるか検証する。

## 参照資料

- [GPUI README](https://github.com/zed-industries/zed/blob/main/crates/gpui/README.md)
- [GPUI Kit README](https://github.com/longbridge/gpui-kit)
- [GPUI Kit Installation](https://gpui-kit.com/docs/installation)
- [GPUI Component TextView](https://gpui-kit.com/component/text-view/)
- [GPUI Base TextView](https://gpui-kit.com/base/text-view)
- [Mermaid追加PR #3685](https://github.com/longbridge/gpui-kit/pull/3685)
- [PR #3685のGitHub API](https://api.github.com/repos/longbridge/gpui-kit/pulls/3685)
- [mermaid-cli](https://github.com/mermaid-js/mermaid-cli)
- [mermaid-rs-renderer](https://github.com/1jehuang/mermaid-rs-renderer)
