# CLIの同一ファイルopenとソース行連携

[設計全体像に戻る](../index.md)

## 対象と根拠

[外部操作](../../requirements/features/external-control.md#ソース行の指定)と[ファイル表示と更新](../../requirements/features/file-viewing.md)を実現する構成を記します。合意した利用者向け契約は要件文書を正本とし、以下の内部型・待機方法は実装上の選択と区別します。

同一DocumentKeyへのCLI openは本文を読み直しません。`mmv open FILE --line N`は保存済みソースの位置を指定します。未保存本文の受信、双方向同期、独自Markdown解析器は対象外です。strict revealを提供する`gpui-base`だけは、承認済みforkの不変Git SHA `1a2539f67f4dbceecb1885c02abd272dbda06e33`へ固定します。その他の依存更新やregistry/cacheの編集は行いません。

## 責務

| 境界 | 責務 |
|---|---|
| cli | 明示的なopenだけに任意の`--line`を受け付け、パスと正のu64を検証する |
| infra / IPC | 単一openを配送し、同一性確認と必要な本文読み込みをUIスレッド外で行う |
| app | 確認後の現在のキーと比較し、継続読み込み・採用・履歴・未適用の行指定を決める |
| core | UTF-8ソース物理行の索引を提供する |
| ui | DocumentAnalysisの共有ASTとsnapshotから実body行へ対応付け、TextViewのstrict API・終端状態・後続操作を接続する |
| runtime | 各段階の要求と結果を配送し、最終完了まで元の応答先を保持する |

## CLIとIPC

`ExternalOperation::Open`は`path`と`Option<NonZeroU64>`の`line`を持ちます。0・負数・非整数・u64超過・欠落・重複指定はCLIの引数エラーです。ソースの行数と比較する前にusizeへ切り捨てません。省略形`mmv FILE`は行指定なしのopenと同じです。`--`、日本語・空白のパス、既存サブコマンドの扱いを保ちます。

プロトコル版は2です。openのopcodeは0の1種類だけで、payloadは長さ付きUTF-16LEの絶対パス、有無1バイト（0/1）、指定時だけ正のu64 LEです。不正フラグ・0・途中欠損・余剰バイト・不正パス・非対応版を拒否します。行専用opcodeと旧形式のdecodeはありません。

フレーム上限、要求ID、ACL、期限、自動再送なし、応答後のreceiptは従来の契約を保ちます。行付きopenにも未起動GUIを起動する権限があります。

成功応答は、同一ファイルでは確認と行指定受理、別ファイル・初回では必要な読み込みと位置要求登録の完了を意味します。解析・レイアウト・画像・Mermaidの完了や画面上の可視性を保証する応答ではありません。

## 確認と読み込みの段階

`ReadRequest`の`ReadPhase::Resolve`と`Load`を区別します。`ReadResponse::Resolved`はキーまたは確認失敗だけを返し、`ReadResponse::Loaded`は実際の本文読み込み結果を返します。空のsourceを未読の代用にしません。

1. CLIのopenとツリーopenは確認段階から開始する。ただし初回の行指定なしopenは現在の文書がないためInitialで直接読み込む。ツリー固有のルート・祖先・葉の安全検査は維持する。
2. appが確認完了時点の現在のDocumentKeyと比較する。同じキーなら本文未読で最終完了する。
3. 異なるキーなら`ReadResolution::Continue`で同じReadRequestIdのLoad要求を返す。確認時点ではCLIへ成功を返さない。
4. Loadでも安全検査とキー解決を行い、実際に読んだ対象のキーを採否に用いる。
5. `ReadCompletion`は最終段階でだけ発行する。古い要求にも確認／読み込みの結果を返し、表示採用の可否とは分ける。

リンク起点の`DocumentOperation::Open`、見出し、履歴、明示Reloadは従来どおり本文を読みます。CLIは`ExternalOpen`で行番号を渡します。

## 独立した要求系列

`DocumentState`はナビゲーションの最新要求、現在の文書期間に対する再読み込み、要求ごとの完了待ちを分けます。`App`は要求IDごとの操作方針を保持します。

- Resolve待ちだけでは本文を読み込み中とせず、保存通知を処理できるようにする。
- 同一ファイルのopen・行指定は、そのファイルの実行中のReloadを取り消さない。
- 表示中Aへの新しいopenが先行Bを追い越した場合、Bの表示採用を失効させてもAのReloadは維持する。
- 別キーの採用成功で文書期間を更新し、以前の期間のReloadを採用しない。A→B→Aでも古いAの結果を復活させない。同じキーでも、後続の実読み込みで本文を採用した場合は、それより古いReloadの採用権を失効させる。古い成功・失敗は応答だけを完了し、新本文やエラー状態を変更しない。未読のCLI確認ではこの失効を行わず、先に受けた実読み込みの採用で後続Reloadを失効させることもしない。
- 読み込み中の保存重複は従来の末尾1回の再読み込みへ集約する。切り替え失敗では元ファイルの保留通知を残す。
- CLIの切断は応答先を解放するだけで、受理済み操作を取り消さない。

## 一度限りの行指定の寿命

行番号は未適用のPositionRequestだけが保持し、現在文書に対する恒久的な行指定状態は設けません。

- 有効な同一openで世代・serial付きの`PositionTarget::SourceLine`を発行する。同じ行番号でも新しいserialを発行する。本文、Entity、検索、履歴、監視、入力、リンク選択パネルは置換しない。
- 行番号なしの同一openは現在の位置と未適用要求を変更しない。
- 読み込み・解析・レイアウト待ちの間は元のu64を保持する。同じ文書のReloadや本文置換では、未適用の見出し・ソース行の要求を新世代へ引き継ぎ、後続の相対移動も保持する。
- 別ファイルの採用成功で旧要求を破棄し、新要求に行番号があれば登録する。失敗では旧要求を残す。
- acknowledgeで対象要求を消す。完了後は元の行番号を保持せず、Reloadでは置換直前の距離を使う。古い世代・serialのacknowledgeは新しい要求を消さない。

## ソースから描画位置への対応

`core/source_position.rs`のSourceLinesはLF・CRLFとUTF-8境界を保ちます。末尾改行だけで外部行指定用の次行を増やさず、空文書は1行です。ソースプレビュー用には従来の末尾空行を残し、プレビューの表示契約を変えません。BOMは既存の読み込み段階で除去します。

1. `DocumentAnalysis`は描画と同じソース・Markdown extensionsで解析した`BlockNode`を再利用し、generationと`RenderedText`の同一性で無効化する。
2. `map_source_line`は通常の文字・表の最初の実text cellをrendered byte positionへ、nestedを含むcodeをsource spanとcode-local offsetへ対応付ける。codeの空行も実際の行を選び、開始／終了fenceは最初／最後のbody行、空codeは実際の空block領域を対象にする。
3. block間空行・表delimiter等は最も近い実body行へ対応付け、同距離なら後のbody行を選ぶ。本文の文字列検索や行割合、高さの推定は使わない。
4. 対応したtargetを`TextRevealTarget`へ1対1変換し、`request_line_reveal`へ渡す。不可視の実renderer計測後、対象の全表示に必要な最小垂直移動だけを原子的に行う。既表示なら移動0、padding・予備移動・block fallbackはない。非常に長いソース行全体を一画面へ収める意味ではない。
5. opaque画像・Mermaid・HTMLや不正構造などは内部の明示失敗にし、隣行／blockへ逃げない。空／空白だけの文書の`Noop`と、最終行超過を既存末尾方針へ渡す`PastEnd`は失敗と区別する。

## レイアウトと操作順

適用前にPositionRequestの世代・serial、TextView Entity、ソース一致を検査します。古いcallbackは新しい要求をacknowledgeしません。

adapterはPositionRequest全体（serialを含む）・generation・TextView Entity・library ID/handle・weak Viewerの非同期waiterを保持します。返却直後の同期terminalも読み、`Shown`／`Failed`／`Cancelled`とmapping失敗をmatching finalizerへ渡します。一致した要求だけを解放し、内部outcomeを保持します。古いterminalは次の要求を消しません。queued movementだけでは再発行せず、同一行の新しい要求には新library IDを発行します。新しいGUI通知は設けません。

strict APIの計測は初回込み8attemptまたは開始から1秒の早い方で終端になります。frameがなくてもtimeoutのwaiterが解放します。成功は最終visible subtreeのpaint後の全矩形包含で判定し、距離の3安定layoutでは代用しません。対応する構成はscrollableかつ行数制限なしのMarkdownだけです。非scrollable／clamped構成は`UnsupportedGeometry`で失敗し、検索・見出しのlegacy APIは維持します。

完了後は本文幅・viewport・画像・Mermaidの寸法変更でも行指定を再発行しません。リサイズは既存の距離保持方針を使います。`bounds_for_item`は上端より前のブロックでNoneになるため、その配列の変化を寸法変更とみなして行指定を復活させる処理は設けません。単なるスクロールによるboundsの変化で終端要求を作り直しません。

UIは要求IDと受付順を対応付けます。同一ファイルの行指定の確認中に受けた相対移動は確認・位置適用の後へ保留し、行指定以前の移動だけを置き換えます。リンクは行／見出しの位置要求が保留されている間は待機し、解除時点の距離を取得して履歴へ渡します。戻る／進むの読み込み要求も、この位置要求の保留が解除されるまではUI側で配送を保留します。要求IDとCLI応答先は保持し、仮のゼロ距離を履歴へ保存しません。

D1として、terminal後のqueued movementがListStateのanchorを変えた場合は、generation・TextView Entity・単調増加movement revisionに対応する正規化barrierを設けます。body Listの正規化済みprepaint後にpaintするcanvasがbarrierを捕捉し、frame後の`cx.defer`で一致したbarrierだけを解放します。古いframe／世代のcallbackや同じviewport寸法では正規化済みとみなしません。受理済みの相対移動を順序どおりまとめて適用し、履歴・read完了・リンク継続は最後の正規化後のlive distanceを取得します。barrier中に受理した操作も同じgateを通します。同一ファイルのidentity-only結果は新位置要求の登録だけを先に行い、後続Backによる取りこぼしを防ぎます。手動のprogrammatic scroll／edge／検索revealはstrictをcancelしてからanchorを変更し、取消しの非同期完了が正規化前に履歴を解放しないようにします。

検証対象・ケース・コマンドと実機確認の境界は[テスト設計](../testing.md#cliの同一ファイルopenとソース行連携)へ集約します。
