# PDF接続契約

[基本設計に戻る](pdf-viewing.md)

## 責務と前提

[PDF要件](../../requirements/features/pdf-viewing.md)、[検索](../../requirements/features/search.md)、[外部操作の終端応答](../../requirements/features/external-control.md#成功失敗と待機)を実現する設計契約である。以下は追加予定のAPIであり、コンパイル済みの実装・既存公開APIではない。

- appは要求採否・文書content・mode・historyを所有し、GPUI／PDFエンジンに依存しない。
- 薄いアプリ側PDF adapterだけがgpui-pdf／hayro型に直接依存する。runtime／UIはadapterのアプリ独自型を使用する。UI側adapterはGPUIへ依存してよい。
- forkは解析・描画・CPU／GPU資源と描画jobを所有する。adapterは型・エラーの変換、接続とcleanup呼出しを担い、解析器や描画器を重複実装しない。
- adapterの`PreparedPdf`、`PdfView`、エラーはprivate fieldを持つopaqueな独自型とする。fork型のtype aliasや再exportではない。大量のtrait／plugin基盤は導入しない。

## 準備・完了の境界

以下の型名・シグネチャは設計上の契約表記であり、Rust定義の全文ではない。

```text
prepare(bytes: Arc<Vec<u8>>, cancel: &PrepareCancellation)
    -> Result<PreparedPdf, PdfPrepareError>
PreparedRead = Resolved { request, key: Result<DocumentKey, ReadError> }
             | Markdown { request, key, source }
             | Pdf { request, key, bytes, prepared: PreparedPdf }
             | Failed { request, error }
             | Abandoned { request }
deliver(result: PreparedRead, viewer, cx) -> ()
from_prepared(prepared: PreparedPdf, host_options, cx) -> PdfView
release(view: &mut PdfView, window: Option<&mut Window>, cx: &mut App) -> ()
finish_internal(request: &ReadRequest) -> bool
```

- infraの途中結果は`Resolved`／`Markdown`／`PdfBytes`／読取失敗を区別する。identity-onlyの既存`ReadResponse::Resolved`は`PreparedRead::Resolved`として明示的に保持し、PDF準備へ流さない。
- `prepare`はbackgroundでforkの実parseと全ページ寸法取得を行う。パスからの再I/O、ラスタライズはしない。非ゼロページ数と有限・正の寸法を成功の不変条件とし、Locked／InvalidDocument／NoPages／InvalidDimensionsは失敗、Abandonedは内部取消として区別する。
- `PreparedPdf`の生成権限はadapterの実prepare成功経路だけに置く。raw bytesを成功と扱わず、偽prepared・空Markdown fallbackを作らない。入力Arcを共有しても全バイトを複製保存しない。
- private constructorを持つ`PreparedRead::Pdf`は同じrequest ID・key・bytesと実preparedを結び付け、一意所有する。runtimeのPDF成功は必ずこの経路を通り、appへ生bytesだけの成功を直接送らない。
- `from_prepared`はpreparedをmoveで一度だけ消費し、再read／再parseせず`ReadyUnlaid`を作る。この入口に新たな回復可能な解析失敗はない。準備失敗は採用前に返す。未採用preparedのdropはCPU資源を解放し、Windowを要しない。

## Resolveから終端またはLoadへの分岐

現在の`DocumentState::resolve`、`App::resolve_at`、`Viewer::complete`のidentity-only経路を拡張し、通常のPDF OpenはCLI・ツリー・リンクともResolveを通す。Markdownの入口固有の既存挙動は変更しない。

1. 登録済みrequest IDと保存したrequest全文・phaseを照合する。不明・改変・完了済みならappを変更せず、保有する結果だけ破棄する。
2. 同一性確認と必要なツリーpreflightが失敗した場合は、通常の失敗で一度だけ終端する。
3. 確認成功後、要求パスの共通形式判定と操作を照合する。PDFへの`--line`は同一キーの成功判定より前に拒否し、Resolveの失敗終端へ接続する。read／parse、位置・倍率・履歴変更を行わない。
4. 同一キーの通常Openなら`adopted=false`の成功で終端する。本文read／parse、view置換、位置・倍率・検索・監視の再初期化、履歴追加はしない。有効なツリー起点の成功では既存の条件付き本文フォーカス移動を維持する。
5. 別キーなら同じIDの保存requestを`Load`へ更新して一度だけ継続する。継続時点では完了通知をemitせず、`pending_operations`も残す。古い外部要求でも実read／parseのoutcomeを返すため継続する。応答義務のない非current要求は下記の内部終端を使える。
6. Loadは読取失敗、Markdown成功、PDF準備失敗、PDF準備成功のいずれかで終端する。古い結果の採用禁止と元IDへの応答義務を分離する。

履歴移動は通常Openのno-op経路とは別である。PDFへの履歴復帰は先頭＋幅合わせとし、保存距離・倍率を適用しない。同一PDFの通常Openをこのリセットへ流さない。

## 採用・通知と状態

| 状態 | 所有者と遷移 |
|---|---|
| Resolving／Reading | appは要求記録、runtimeはI/O job。旧文書を維持する |
| Preparing | runtime registryのjobがbytesと取消tokenを保持し、adapterがfork prepareを呼ぶ |
| Prepared | 配送envelopeが実preparedを一意所有。まだ表示・履歴を変更しない |
| ReadyUnlaid | appの採用と同じforeground更新内でUIへ一度だけmoveし、初期layoutを待つ |
| Active | forkがviewport近傍を描画。appの要求IDとは別の描画epochで選別する |
| Failed／Abandoned／Released | 各所有者が終端回収。Releasedから復帰しない |

`deliver`はID・保存request・phaseを再照合し、実preparedを保持したままapp中立の成功／失敗へ変換する。appが現在要求と採否を決め、採用されたときだけ同じforeground更新内でapp／history採用→UIの`from_prepared`とview交換→標準`ReadCompletion` emitの順とする。UI交換は再解析しない。旧viewを解析中に解放しない。

非採用成功・失敗は元IDの実outcomeを一度だけ通知し、preparedはdropする。失敗・非currentでは旧表示・位置・倍率・history・入力を変更せず、current失敗だけ既存のエラー処理へ接続する。CLI成功は準備までを条件とし、layoutや全ページrender完了を待たない。

## 内部取消と要求記録の回収

- runtimeのforeground registryがIDごとにtoken原本、job handle、応答義務、配送状態を所有する。背景jobへtokenをcloneし、取消権限はregistryへ集約する。tokenのdropを暗黙cancelにしない。
- 新要求による非current化だけでは、外部応答義務のある要求を取消しない。CLI切断・タイムアウトも受理済み操作の取消ではない。実read／parse outcomeを既存終端応答へ返し、staleを理由とする独自cancel errorを追加しない。
- 応答義務がなく非currentとなった要求に限りcancelする。未開始queueを除去し、read後／parse前／寸法取得後にflagを確認する。`Abandoned`は結果をdropするだけでなく、app中立の最小終端入口`finish_internal`へ配送する。
- `finish_internal`は保存request全文・ID、応答義務なし、非currentを確認し、`DocumentState::outstanding`と`pending_operations`の該当記録を同一更新で一度だけ除去する。表示、position、倍率、history、error、mode、現在のpending／reload要求を変えず、`ReadCompletion`をemitしない。該当しなければfalseで状態不変とする。通常の`finish_operation`による位置／履歴処理へは流さない。
- registryも同じIDを終端済みにする。unknown／duplicate／late resultは再emitも二重回収もしない。phase変更前の遅着ResolveでLoad記録を消さない。UIのread order／配送待ちenvelopeも該当IDだけ回収し、現在要求の順序を変えない。
- 同期hayroの強制停止は保証しない。取消不能jobはViewerに所有させずruntimeの収集taskが最後までawaitし、WeakEntityが消滅していても結果・bytes・tokenをdropする。registry除去と同時に収集taskまで失わない。
- window消滅時は新配送を止め、取消可能jobを止める。外部応答義務は既存`PipeConnection::stop`と同様にpending replyを解放してserverをshutdownし、接続終了として扱う。成功や独自のPDF取消応答を捏造しない。appも消滅する場合は要求mapをdropし、生存するappを閉じる場合は、現在要求を失効させたうえでshutdown専用の中立なdrainで両mapを回収する。このdrainは表示・history・errorを変更せず、通常稼働中の`finish_internal`の条件を緩めない。取消不能jobの収集責任はruntimeに残す。
- アプリ終了時も収集taskをjoin／回収してから所有権を閉じる。未完jobがあれば即時解放済みと報告しない。準備jobとforkの描画jobは別registryとする。

## view操作とrelease

adapterの`PdfView`はfork viewを私有し、`scroll_by(logical_y)`、`scroll_viewport(fraction)`、`scroll_to_edge`、`set_zoom(factor)`、`fit_width`を公開する。正は下、viewportは本文基準とする。有限値以外はInvalidArgumentで状態不変、Released後はReleased error、有効操作はApplied／Unchanged／DeferredUntilLayoutを返す。可変ScrollHandleは公開しない。

ReadyUnlaidのviewport依存操作は世代付きintentとして保持し、有効layoutで一度だけ適用する。移動はclampとnotifyを伴い、ズームは中央のページindex＋正規化座標を保つ。`fit_width`はtoggleではなく冪等、手動倍率変更でfitを解除する。入力条件と刻みは[PDF要件](../../requirements/features/pdf-viewing.md#スクロールとズーム)を正本とする。

`release`は冪等で、forkに新spawn停止、epoch失効、queue破棄、callback解除、job取消、画像・文書・bytesの解放を一括委譲する。置換・closeともadapter経由で呼ぶ。更新中のWindowはSomeで渡し、Windowなしの終了もforkのrelease hookで処理する。描画中の取消不能jobはforkが回収し、遅着結果を採用せず資源をdropする。単なる画像消去を完了としない。

## Luaズームの接続

公開契約は[PDFズーム仕様](../../requirements/features/pdf-zoom.md)を正本とする。依存revisionは変更せず、既存の公開操作だけを使う。

- Luaは引数・phaseを検証し、採用時に公開された表示identityをtyped host requestへ保持する。ホストは現在表示とのidentity一致を確認してadapterへ渡す。パス一致だけで旧要求を再利用しない。
- adapterは未適用のズーム操作をFIFOに保持する。相対倍率は先行操作の適用後に評価する。幅fitはhost用の冪等API、ページfitはネイティブtoggleへ委譲し、位置・resize・微小倍率差の判定は依存に任せる。
- adapterの描画後に、公開のゼロ距離scroll操作で内部viewportの有効性を確認する。未layoutなら相対zoomを依存側の絶対倍率intentに変換せず保留する。有効になった初回に幅fitを確立し、保留操作を順に適用する。
- Lua向けread modelはadapterだけが更新する。初回有効layoutまではnil、適用後は依存の実zoomを公開する。描画後の更新で自動fitによる変化も反映し、受理時点で未来値を作らない。
- releaseは保留queueと公開値を失効させる。表示変更・owner callbackを必要としない受付専用APIであり、新規結果配送や通知の仕組みは持ち込まない。

## 実装段階と所有範囲

| 段階／予算 | 境界と受入れgate |
|---|---|
| T2a＋T2b／合計60分 | T2aはcontent enum／accessorと既存MarkdownのI/O・UI・fixtureを移行。source二重保持・空Markdown fallback・PDF成功入口は禁止。実compile・Markdown回帰・独立review・受入れを先行gateとする。時間内にcheckpointし未達なら停止・再見積り |
| T1／60分 | forkの解析・prepared／view操作・host入力分離・資源所有。fork Cargo唯一所有者。T2a受入れ後にT2bと並列可 |
| T2b | app/core/infraのengine-neutral形式・生bytes・history/search/watch方針。fork型・PDF prepare・Cargo・UIは扱わず、Markdown production配送を維持する |
| T3／60分 | 実adapter／runtime配送・PDF view・共通UI接続。app Cargo／lock唯一所有者。T1／T2b受入れ後に実parser／GPUI／CLI結合を確認 |
| T4／60分 | 操作・入口・既存統合。T3編集停止・独立review・受入れ後にUI所有権を直列移管し、同じUIを並列編集しない |
| T5／30分 | 空き枠で専用docs/scripts/検体だけを準備。共有src・test登録・Cargoは禁止。PDFは自作しOFLフォントの条件を確認する。T0では検体を作らない |
| T6／45分 | T1–T4受入れとT5準備後の横断機能・Windows/IME・性能・資源検証。修正は所有taskへ戻す |

T2aのgateは`cargo check --locked --bin mmv`、`cargo test --locked --bin mmv`と既存ui-testsのsearch／modes／history／operations／strict revealとする。T2bもfork未完了変更なしでMarkdown回帰と中立方針テストを実行する。各段階が自身の検証と別worker reviewを持ち、T6へ押し付けない。最大2workerとし、予算は成功保証でなく停止・再計画のcheckpointである。

## 検証観点

- Resolveの同一／別キー／失敗／PDF --line、Load継続の同IDとphase、通常no-opと履歴top-fitの区別。
- 実prepare以外ではPDF成功を生成できず、一回のread・parse・move・completionとなること。
- 逆順成功／失敗、内部取消の両map回収、外部応答維持、duplicate／unknown／旧phaseの状態不変。
- 置換・window消滅・終了後の取消不能prepare／raster結果の回収と、Windowあり／なしの冪等release。
- Search確定A・編集BからPDF上でIME後Enterを押した終端と、Search継続でMarkdownへ戻る分岐。

Send／Sync、GPUI 0.3.11互換性、実shutdown、GPU解放、IME、性能は実装段階のbuild・結合・実機検証を要し、本契約を記述しただけでは実証されない。
