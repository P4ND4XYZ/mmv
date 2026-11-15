# PDF検証の準備とT6実施手順

## 範囲

T5は再生成できる自作検体と検査・計測手順だけを準備します。アプリの表示・IME・性能・資源解放を検証した報告ではありません。[要件](../requirements/features/pdf-viewing.md)、[設計](../design/components/pdf-viewing.md)、[接続契約](../design/components/pdf-viewing-contract.md)を正本とします。**性能の数値合格閾値は未決定**です。ここでは測定値や仮の合格値を作りません。

## 生成と検査（Windows / Python 3.12、標準ライブラリのみ）

リポジトリルートから実行します。インストールや依存追加は不要です。初回のフォント取得だけネット接続が必要です。

```powershell
python scripts/generate_pdf_fixtures.py --help
python scripts/generate_pdf_fixtures.py --output target/pdf-fixtures/corpus-a --download-font
python scripts/check_pdf_fixtures.py target/pdf-fixtures/corpus-a
python scripts/generate_pdf_fixtures.py --output target/pdf-fixtures/corpus-b
$env:PYTHONPYCACHEPREFIX = "$PWD/target/pdf-fixtures/pycache"
python -m py_compile scripts/generate_pdf_fixtures.py scripts/check_pdf_fixtures.py scripts/test_pdf_fixtures.py
python scripts/test_pdf_fixtures.py -v
```

`--assets` は既定で `target/pdf-fixtures/assets`。不変revisionのフォントを `font-source.json` の容量・SHA256で照合し、違えば失敗します。保存済みOFLのSHA256も照合します。`--download-font` なしでキャッシュがなければ失敗し、既存の不一致フォントを再取得・上書きしません。第三者PDFはダウンロードしません。

生成先・assetsは `target/pdf-fixtures` の配下限定で、リンク・ジャンクション経由も拒否します。出力ディレクトリは新規必須で、既存ファイルを消しません。I/O失敗時は部分出力が残る可能性があります。ログと対象を確認して新しい出力先を使い、自動削除・無条件再実行をしないでください。競合する他プロセスとの原子的隔離は提供しません。

生成・検査は成功時に終了コード0とJSON、引数・I/O・同一性・検査失敗は終了コード2とstderrを返します。検査器はこのwriter固有のclassic xref、オブジェクト参照、stream長・展開、ページツリー、画像サイズ、フォント同一性、ハッシュを調べます。汎用PDF構文検証器・実アプリの解析器や描画器の代わりではありません。不正・切断検体は `expected-invalid` / `expected-truncated` と分け、正常な解析成功とは扱いません。修復可能なPDFをエンジンが受理する場合もあるため、T6で実エンジンの結果を記録してください。

テストはCLI・ファイルを実物で接続し、ネット・GUI・本番アプリは起動しません。既定assetsに検証済みフォントを置いてから実行します。一時出力は専用target配下で後始末します。対象外パス、上書き、リンク／ジャンクション、欠落／改変フォント、改変PDF、manifestの不正形状／分類の偽装、xref破損も検査します。検査時の生成スクリプトとmanifestの生成元SHA256が違う場合は再生成を求めます。

## 検体と再現性

| ファイル | ページ | 内容・目的 |
|---|---:|---|
| normal.pdf | 1 | 自作ASCIIテキストと枠、通常の基準 |
| japanese.pdf | 3 | 自作の意味ある日本語、横書き、かな・漢字・句読点、無改変Noto Sans JP埋め込み。各ページは同じ本文 |
| image-heavy.pdf | 12 | ページごとに異なる自作512×512 RGBラスター1点とベクトル図形。写真・スキャン・JPEGではない |
| many-pages.pdf | 300 | 自作ASCIIページ番号と枠。大規模な混在レイアウトではない |
| malformed.pdf | 0（意図） | PDFヘッダーもない不正文書 |
| truncated.pdf | 1（意図） | normal.pdfの最初の96バイト。ページオブジェクト・xref・EOFを欠く |

固定された整数演算で画像を生成し、乱数・日時・絶対パスをPDFに入れません。同じスクリプト・フォント・zlibならPDFが一致します。Python/zlibの別バージョンでは圧縮差により同じ見た目でもバイト列が変わる可能性があります。manifestはPython版、zlib版、生成スクリプトSHA256、パラメータ化したコマンドを記録します。再生成時には各PDF、OFL、font-source、manifestのSHA256を比較し、差があれば原因を記録してください。フォントの可変軸を変更せず既定outlineを使用します。

日本語の構造検査では全使用文字のUnicode cmapが非0 glyphに対応し、CID=元glyph ID、明示幅とToUnicode、フォント全体のSHA256を確認します。**これだけでは見た目の正しさを証明しません**。独立したWindows PDF解析・オフスクリーン描画で日本語の欠字・崩れ、画像色・枠・クリッピングを別途確認し、使用API・OS・対象hash・結果を記録してください。実アプリでの描画はT6で再確認します。

## T5の描画確認と成果物

準備環境はPython 3.12.0 / zlib 1.2.13 / Windows NT 10.0.26300.0です。Windows.Data.Pdf.PdfDocumentで正常4検体のページ数と不正2検体の拒否を確認し、Windows標準PDF APIのオフスクリーンPNGで日本語本文の意味・欠字なし・画像のグリッドと色を確認します。これはアプリのhayro/GPUI描画成功を意味しません。最終検体は専用作業領域の `target/pdf-fixtures/corpus-r2`、再生成は `target/pdf-fixtures/repeat-r2` に保持します。各検体identityは同ディレクトリのmanifestに記録します。後続作業領域へ移す場合はmanifestとOFL・font-sourceを一緒に保全し、hashを照合してください。

## T6記録票（未実施）

まず専用の検証環境とプロセス識別方法を定めます。利用者の既存mmv GUI／既定singletonへ接続しません。隔離起動方法が用意できなければ停止し、未確認と記録します。

記録する環境:

- 日時、Windows版・build、CPU・RAM・GPU・ドライバ、ディスプレイDPI／解像度、電源・負荷条件。
- アプリ／forkのrevision、作業ツリー状態、ビルドprofile・コマンド、依存lockのidentity、exeのSHA256。
- corpus絶対パス・manifest SHA256、各PDFのSHA256・容量・ページ数・内容特性。
- ウィンドウ／本文viewportの論理px、初期ズーム、ツリー表示、入力モード／フォーカス、キャッシュがcold/warmのどちらかとその定義。
- 計測ツールと版、時計の起点・終点、サンプリング間隔、反復回数・順序・休止条件を実施前に記録。平均だけでなく各試行と外れ値・欠測理由を残す。

### オープンと初回表示

CLI、ツリーEnter／l、MarkdownのローカルPDFリンク、大小拡張子を確認します。CLI要求送信、応答受信（解析完了）、レイアウト適用、最初の本文可視化の時点を分けます。CLI応答を全ページ描画完了や初回可視化と取り違えません。通常→日本語→画像多数→300ページをcold/warm条件別に開きます。同一PDFの再openは位置・倍率を変えず、別文書は同じ領域へ切り替わることを記録します。

### スクロール・ズーム・切り替え

- 本文Normalでj/kの40論理px、Ctrl+d/uの半viewport、Ctrl+f/bの全viewport、gg/G、ホイールを両方向に実施。入力時点と位置変化・新しい描画が見える時点を分離し、描画待ち・空白・古い画像・キー二重実行を記録。
- 各検体で先頭→末尾→先頭の反復、画像文書↔300ページ↔Markdownの反復切り替え。回数・間隔・順序を固定して記録し、メモリ推移を時系列で残す。有限キャッシュの初期増加と増え続ける傾向を区別。
- `plugins/pdf_zoom.lua`を明示ロードして+/−の×1.1／÷1.1、=の冪等な幅合わせ、pの最上部ページfit toggleを確認。未ロードではズームキーが無割当であることも確認。中間位置の保持、resize・ツリー切り替えへの追従、手動zoom後のfit解除を確認する。ページfitの中央配置・全体可視性は要求せず、resizeは現在の最上部ページを使い、toggle解除時は倍率を保持する。

### 資源と失敗経路

対象PIDとその子だけを計測し、CPU、private bytes／working set、GPU dedicated/shared memoryとページ画像・job状況を関連付けます。開く前、解析中、反復中、文書置換後、close後、終了後を同じ方法で記録。Windowあり／なしの終了、遅着描画・解析jobの回収が終わるまで解放済みと判断しません。専用プロセス以外は停止しません。

不正／切断、読取不能・不存在、PDFへの--line（同一openも含む）、交錯するopenの逆順完了を試します。エラー応答と表示・位置・倍率・履歴・入力modeの保持を確認。Markdownへfallbackしないことを確認します。失敗検体が修復受理された場合は実際の分類を記録し、成功に見せるため期待値を変更しません。実施後にPDF hashを再照合します。

### Windows実画面専用（オフスクリーンとは分離）

IME・OSフォーカス・キーroutingは実画面で人が確認します。PDFクリック→本文Normalの操作、Search／Command／ツリー／選択パネルの文字入力を保護し、内蔵ヘッダー・検索・キー操作と二重実行しないことを確認します。

Searchの確定語A・編集中Bから外部PDF切り替え、IME未確定Enterの保護、確定後EnterでBを確定せずAを保持してNormal＋INPUT通知、Search継続でMarkdownへ戻る場合はB優先を分けて記録。PDFの/・n・N通知、F5非再読込、履歴復帰の先頭＋fit、Markdownの従来検索／位置復元も確認します。

## 結果の扱い

各項目を「実施・問題なし」「問題あり」「未確認」で分け、再現手順・対象revision/hash・計測条件・生の測定値と観察を記録します。オフスクリーン構造／描画成功を実GUIのIME・性能・GPU解放の成功へ読み替えません。閾値の採用や実装調整は測定結果を基に別途判断します。
