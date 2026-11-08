# 技術方針と採用前の検証

[要求・要件](../requirements/index.md)

## 技術方針

### 描画基盤

GPUIとgpui-componentを用いた既存の[プロトタイプ](../design/prototypes/gpui-evaluation/index.md)を流用して、ビューアーを作成します。

### 外部操作

- CLIを外部アプリからの共通の入口にする。
- CLIから起動済みビューアーへの通信には、WindowsのNamed Pipeを使う。
- 単一起動を前提とし、操作対象のインスタンスを利用者が選択する仕組みは設けない。
- 未起動時のスクロールなどは、呼び出されたCLI側で接続失敗を検知し、失敗を返す。未起動のビューアー自身に検知させる方式ではない。

コマンド構文、パスの解釈、応答待機と終了コードは[外部操作のCLI仕様](../requirements/features/external-control.md)に従います。

### Mermaid

初期版はプロトタイプのmermaid-cliによる非同期生成を流用し、永続キャッシュは追加しません。事前インストール、失敗時の非表示、一時生成物の整理は[Mermaid図](../requirements/features/mermaid.md)、ログ保存とローテーションは[エラーログ](../requirements/features/logging.md)に従います。永続キャッシュの再利用・容量管理は後続版の対象です。

### Mermaidキャッシュの利用日時

以下は後続版の永続キャッシュに用いる方針であり、初期版では実装しません。

- 図の画像ファイルとは別に、同じキャッシュキーに対応するJSONファイルへ最終利用日時などのメタデータを保存します。
- 表示に使った際はJSON側の日時を更新し、画像本体を変更する必要はありません。
- OSのファイルアクセス日時は、更新が無効・遅延している場合があるため、削除順の判定には使いません。
- 利用とみなすタイミング、容量上限、削除順と期間の閾値を設けない方針は[Mermaid図](../requirements/features/mermaid.md#最終利用日時)で定めます。

プロトタイプの図の生成形式はPNGです。

## GPUI / gpui-componentの予備調査

プロトタイプ作成前の調査結果と、当時の検証候補を示します。

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

## プロトタイプの検証結果

- [初期実装の検証結果](../design/prototypes/gpui-evaluation/results-initial.md)
- [Mermaidの検証結果](../design/prototypes/gpui-evaluation/results-mermaid.md)
- [モードUIの検証結果](../design/prototypes/gpui-evaluation/results-modal-ui.md)

製品への流用方針と実装順序は[ビューアーの詳細設計](../design/index.md)を参照してください。

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
