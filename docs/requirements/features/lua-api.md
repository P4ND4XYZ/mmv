# Lua公開API

[全体像に戻る](../index.md)

## 目的と位置づけ

[LuaによるフローティングUI](lua-floating-ui.md)を利用するための、追加する公開APIの入力・結果・順序・寿命を扱います。現行Rust実装のAPI一覧ではありません。既存の`mmv.setup`・`mmv.set_hl`は[表示・配色設定](display-settings.md#lua-api)を参照してください。`local mmv = require("mmv")`でAPIを取得し、`local ui = mmv.ui`でUI APIを取得します。

## 共通規則

- optionsは名前付きtableで渡す。不明なパラメータ、不正な型・値はLuaエラーとする。
- 構成APIが返すものは表示記述であり、windowや部品の操作用オブジェクトはRustのハンドルとする。GPUIのEntityや描画コンテキストは公開しない。
- renderとrender_itemは表示記述を1個返す。配列・nilは返せない。
- 文書・ソース・遷移先は読み取り専用のハンドルとする。Luaへ世代番号は公開しないが、内部の古い結果・対象の検証は維持する。
- renderは表示記述を返す処理とし、`state:set`、window・部品操作、遷移などの副作用を禁止する。
- 設定・選択・focus・scroll・通知の操作は、要求を受理したら`true`を返す。実際に表示が変わった保証ではない。個別の`false`条件は各APIの規則に従う。
- `doc:links()`の配列・候補レコード、イベント・結果のtableは独立したコピーとし、Lua側で変更してもホスト内部を変更できない。

### 入力型・省略・変換

- string・boolean・functionはそのLua型だけを受け付ける。数値から文字列、truthy値からboolean、callable tableからfunctionなどへの暗黙変換はしない。
- 表示文字列・message・キー文字列・fuzzyのquery/textは有効UTF-8とする。この表示入力の制約を、stateに格納する一般のstringへは広げない。
- 必須項目のnil・欠落はLuaエラー、任意項目のnilは省略扱いとする。
- `keymap.set/remove`・`navigation.open`・`notify`・`scroll_to`のoptions省略は空table相当。`float.open`・各部品の構成table、`source:scroll`のpagesは必須とする。
- `render`・`render_item`・`button.on_click`・`keymap.set`のcallbackは必須function。その他のイベントcallbackは任意functionで、省略時は何もしない。ただしon_error等のホストfallbackは各節に従う。
- 表示構築関数を除くイベントcallbackの戻り値は使用しない。callbackの作用はstate更新や操作APIで表す。

## 初期化と設定の有効化

標準のキー登録を行ってから利用者設定を適用します。利用者分の設定・登録は成功時に一括で有効化し、失敗時は標準設定・標準キーを維持します。設定を部分適用しません。設定失敗時の通知・ログは[表示・配色設定](display-settings.md#設定に失敗した場合)に従います。

initで利用可能なのは`setup/set_hl`・keymap・state、純粋な表示記述作成（render内限定のlistを除く）・fuzzyです。document参照・購読、notify、window表示・遷移は実行フェーズまで利用できません。`ui.state`の生成・取得・更新は許可します。

`setup/set_hl`はinit専用、keymap変更は実行フェーズでも可能です。設定・キー登録APIの検証エラーは`pcall`で捕捉しても初期化全体を失敗させます。それ以外の捕捉済みエラーへ、この初期化全体失敗の扱いを広げません。

## キー登録と配送

```lua
mmv.keymap.set("f", open_links, { mode = "normal" })
mmv.keymap.remove("f", { mode = "normal" })
```

`set`のcallbackは引数なしです。登録条件は`mode`だけとし、`focus`条件は設けません。初期版の対応modeは`"normal"`のみで、既定も`"normal"`です。修飾キー付きも含む単一キー操作を扱い、連続キーとSearch/Commandの文字入力上書きは提供しません。標準のリンク選択も同梱Luaで登録し、利用者が上書き・解除できます。`keymap.remove`は登録がなければ`false`を返します。

### キー文字列と重複

- `ctrl`・`shift`・`alt`・`super`を`-`でつないだ修飾部と、キー1個で表す。superはWindowsキー。修飾順は正規化し、同じ修飾の重複はLuaエラーとする。
- 修飾名・名前付きキーは大文字小文字を区別せず小文字へ正規化する。別名は設けない。
- 通常キーは空白・制御文字を除く1 Unicodeコードポイント。キー部分のASCII大文字A〜ZはLuaエラーとし、`shift-a`などで明示する。大文字からshiftへ自動変換しない。
- 名前付きキーは`enter`、`escape`、`tab`、`space`、`backspace`、`delete`、`insert`、`home`、`end`、`pageup`、`pagedown`、`left`、`right`、`up`、`down`、`f1`〜`f24`。空文字・連続キー・不明名はLuaエラーとする。
- 修飾部を除いた残りが`-`一文字ならハイフンキーとする。例は`-`、`ctrl--`。
- 一つのkeys表で正規化後のキーが重複したらLuaエラーとする。`keymap.set`の同mode・同キーへの再登録は上書きする。

### 配送と既定キー

window全体の`keys`と、フォーカス中の`ui.input.keys`を提供します。`keys = { ["ctrl-u"] = function() ... end }`はキー文字列から関数への対応表であり、配列や連続キーの表現ではありません。専用の`navigate`オプションは設けず、Luaのcallbackがlist/sourceハンドルを操作します。

配送順はIME、フォーカス中の部品、所属window、appです。ただしmodalの背後には送りません。各層の追加keysは同じ層の既定操作を上書きできます。該当callbackを呼んだキーは消費し、callbackの戻り値やエラーによって背後へ流しません。IMEは常に最優先とし、変換確定のEnterではsubmit・候補確定を行わず、変換中のEscは変換取消を優先します。

追加割り当てがない場合の部品既定キーは次のとおりです。有効な既定キーは、端などで無作用になっても消費します。

| 対象 | 既定動作 |
|---|---|
| input | 通常の文字編集、矢印、Shift選択、Home/End、Backspace/Delete、Ctrl+A/C/X/V/Z/Y。Enterはon_submit、Escはwindowへ渡す。IME保護を維持する |
| list | ↓/Ctrl+nで次、↑/Ctrl+pで前、Enterでconfirm。候補クリックはそのIDを選択・確定し、古い候補は確定させない |
| source | ↑/↓は表示上の1行、PageUp/Downは1ページ、Home/Endは先頭/末尾へ移動。本文側へ伝播しない |
| button | Enter/Spaceでon_click |
| window | Tab/Shift+Tabでwindow内を循環。Escはdismiss.escapeが有効な場合だけ終了操作となる |

inputから別list/sourceへの操作は自動接続せず、Luaのkeysで明示的に接続します。

## フローティングウィンドウ

`ui.float.open(options)`はwindowハンドルを返します。

| 項目 | 契約 |
|---|---|
| `width` / `height` | 必須。有限の正数の論理px、または`{ ratio = ... }`。比率は`0 < ratio <= 1` |
| `placement` | `"center"`。既定も`"center"` |
| `modal` | boolean。既定`true` |
| `focus` | 開いたときだけフォーカスを与える部品key。再描画では戻さない |
| `render` | 必須。引数なしで表示ツリーを返す |
| `dismiss.escape` / `dismiss.outside_click` | booleanで、それぞれ既定`true`。dismiss全体省略でも両方`true`。modalかどうかとは独立 |
| `keys` | window内の追加キー割り当て。省略時は空 |
| `on_close(event)` | 終了後に一度呼ぶ |
| `on_error(err)` | 未処理のevent/renderエラーを受ける |

floatの比率はアプリ全体の描画領域を基準にします。floatがその領域より大きい場合は、描画領域内に収めます。

`on_close(event)`のeventは`{ reason = "programmatic" }`の形式です。終了理由は`programmatic`、`escape`、`outside_click`、`shutdown`です。本文置換通知からLuaがcloseした場合も`programmatic`です。

### 重なり・フォーカス・終了操作

- 新しいwindowを最前面へ置く。既存floatのクリック・`win:focus`ではz順を変更せず、昇格APIは追加しない。最前面のmodalより下の利用者操作を遮断し、そのmodalより前面のwindowは操作できる。
- 文書読み込み・CLI・画像・Mermaidなどの非同期処理は止めない。
- nonmodalの背後はクリックで操作・フォーカス移動できる。背後にフォーカスがあれば、その対象のキー操作を有効にする。
- 外クリックによって終了した場合、そのクリックは消費し、背後へ通さない。終了操作は最前面の対象だけに作用し、1回のEscで複数を閉じない。
- フォーカス対象はinput・list・source・有効なbuttonとし、row・column・textは対象外とする。開いているwindowで対象外の部品を`focus`または`win:focus`に明示指定した場合はLuaエラーとする。閉じたwindow・失効した部品への操作は、通知とエラー・失効の規則に従い無作用で`false`とする。
- Tab/Shift+Tabはmodal/nonmodalともwindow内の対象を表示順に循環し、対象がなければwindow自体に留まる。背後に既にフォーカスがあるnonmodalでは、背後のキー操作を維持する。
- 最前面modalより下への`win:focus`は受理せず、無作用で`false`を返す。後で自動実行するために保留しない。閉・失効のfalseと、有効対象の不正key・非focus対象のLuaエラーは維持する。
- フォーカス中の部品が削除・無効化されたら、同じwindowの表示順で次の有効部品へ移す。末尾なら先頭へ戻り、対象がなければwindow自体へ移す。
- `focus`省略のnonmodalは現在のフォーカスを維持する。modalはパネル自体でキーを受け、背後へ送らない。
- 閉じるwindow内にフォーカスがあった場合だけ、直前の有効な対象へ復帰する。OSのフォーカスを奪わない。

### 生成と初期イベントの順序

1. `open`内で初回render、定義検証、部品作成を行う。
2. `open`が戻った時点で、部品を取得・操作できる。
3. 初期選択などのcallbackは、呼び出し元のLua処理が終了してから配送する。
4. レイアウトを必要とするスクロールは、レイアウト後に一度適用する。

初回render・検証失敗は呼び出し元へのLuaエラーとし、不完全なwindowを残しません。生成途中のwindowから重複通知しません。初回render中は`open`の戻り値がまだ代入されていないため、そのwindowハンドルを参照せず、後のcallbackで使用します。

### windowハンドル

| 呼び出し | 動作・結果 |
|---|---|
| `win:close()` | 初回の終了要求受理は`true`。繰り返しても追加の作用はなく`false` |
| `win:is_open()` | 開いているかをbooleanで返す |
| `win:focus(key)` | 指定部品へフォーカスを移す |
| `win:list(key)` | list操作用ハンドルを取得する。存在する部品の型が異なればLuaエラー |
| `win:source(key)` | source操作用ハンドルを取得する。存在する部品の型が異なればLuaエラー |

一覧・ソース固有の操作をwindowにも重複して提供しません。取得・生成APIはそれぞれ所定の値だけを返し、余分な戻り値を追加しません。閉windowの`win:list/source`は既存寿命規則どおり`false`です。非wrap端・同じ選択・`scroll.pages = 0`でも、有効な要求なら受理結果は`true`です。

## callbackの所属と配送

- 所属は登録元で決める。windowのrender/render_item/keys/部品イベント、およびそのwindowをownerとする文書購読・遷移callbackはwindow所属。app keymapとownerなし遷移callbackはwindow外とし、closureがwinを参照するだけでは所属を変えない。
- render_item失敗はphase=render、その他のイベントcallback失敗はphase=event。関連部品を特定できる場合だけkeyを付ける。on_close失敗・on_error自身失敗のホスト送りは維持する。
- イベントcallbackは実行中のLua処理へ割り込ませず、その処理終了後に配送する。open初回のrender/render_item等、表示構築のため契約上明示した同期呼び出しは除く。API内の同期renderを禁止する規則ではない。
- 利用者の確定編集は、bind state更新→on_change→正常表示採用/選択補正→on_selection_changedの順とする。on_change内の更新もその表示反映へ含める。プログラムsetではon_changeを発行しない。
- 同一window内で複数の選択通知が必要なら表示順。独立window間の通知順は公開保証しない。
- 終了で未実行の通常callbackを無効化するが、実行中callbackを途中中断しない。on_closeは終了通知として終了後一度呼ぶ。実行中にownerが閉じた後の未処理エラーはホスト通知・ログへ送る。

## 状態と表示記述

```lua
local query = ui.state("")
local value = query:get()
query:set("new")
```

`ui.state(initial_value)`は検索キーによる取得ではなく、新しい状態の入れ物を生成します。`ui.state()`はnil初期値相当です。renderの外で生成し、`get`で値を読み、`set`で表示更新を通知します。`set`は同値でも更新を通知し、同じcallback内の更新はまとめて反映します。

許容値はnil・boolean・有限のnumber・string・循環しない通常のtable、およびmmvの読み取り専用の文書・ソース・遷移先・範囲ハンドルです。function・thread・上記以外のuserdata・metatable付きtableは拒否します。範囲ハンドルを含むリンク候補もstateに保存できます。

state内のtableキーは文字列・有限数値・booleanを許可します。初期値・`set`・`get`の境界でtableをコピーし、コピー内部では同じ子tableへの共有参照を維持します。取得したtableの編集は、`set`するまでstateに影響しません。読み取り専用ハンドルは共有し、文書全文をコピーしません。

`input.bind`はstring stateとの双方向接続、`list.selection`は候補IDまたはnilのstateです。初期版では、1つのstateを同時に双方向bindできる部品は1個までとし、複数部品からの`get`は許可します。bind先の型に合わない`set`はLuaエラーとし、元の値を保持します。

textなどにはrender内で`get`した値を渡します。検索結果は候補とqueryから導出でき、別stateを必須としません。IME未確定範囲・カーソル・スクロールはRustが保持します。

### 共通の部品規則

- `key`は任意の文字列でwindow内一意。状態を持つ部品では必須とし、別windowの同名keyは許可する。
- 同じkey・同じ型では入力・IME・フォーカス・スクロールを維持する。削除や型変更では部品状態を破棄する。
- `children`・`items`・`highlights`は1始まりの欠番のない配列とし、空配列を許可する。追加の文字列キーやmetatableは許可しない。
- 部品の`width` / `height`は有限の正数の論理px、`{ ratio = ... }`、文字列`"auto"`を扱い、省略時は`"auto"`とする。比率は`0 < ratio <= 1`で、親のpaddingを除いた内容領域を基準にする。
- `grow`・`gap`・`padding`は有限の非負数とする。`grow`は残余領域の分配重みで、既定`0`。`padding`は全辺共通で、既定`0`。
- `hl`は[既存の配色グループ名](display-settings.md#mmvset_hlname-attributes)を指定する。

### row / column / text

`ui.row`・`ui.column`は必須の`children`を並べます（空配列可）。`gap`の既定は`0`。`align`は並べる方向と直交する方向の配置に使い、`start` / `center` / `end` / `stretch`を受け付けます。既定は`stretch`です。

`ui.text`はstringの`text`必須、booleanの`wrap`の既定は`false`です。`highlights`を省略した場合は強調しません。指定する場合は次の形式の要素を渡します。

```lua
{ range = { start = ..., stop = ... }, bold = ..., underline = ..., hl = ... }
```

highlightsのrange.start/stopはLua integer、bold/underlineは任意booleanです。未指定属性は基底styleを継承し、明示falseは装飾解除、hl省略は基底配色を継承します。hlは既存配色group名です。

textの範囲は、その表示文字列内の0始まりUTF-8 byte・半開区間です。文書内のソース範囲とは別物です。範囲外・逆順・UTF-8文字途中の境界はLuaエラーとします。空範囲は許可し、表示には影響させません。範囲が重なる場合は、後の要素で明示した属性を優先します。

### list

`ui.list`はrender内でのみ使用します。`key`、`items`、`selection`は必須で、itemsの各項目には一意の`id`を持たせます。IDは文字列またはLuaの`integer`型です。文字列との暗黙変換はしません。各項目は通常tableで、id以外のpayloadはstate許容値とし、重複IDはLuaエラーです。

| 項目 | 契約 |
|---|---|
| `render_item(item, selected)` | 必須function。itemとbooleanのselectedから表示記述を1個返す |
| `empty` | 候補がない場合の任意の表示記述1個。省略時は何も表示しない |
| `reset_on` | nil・boolean・有限number・stringに限定し、値の比較で変更を判定する。変更時に先頭へ選択を戻す |
| `wrap_navigation` | boolean。既定`true` |
| `on_selection_changed(id)` | 選択IDの変更を通知する。選択なしはnil |
| `on_confirm(id)` | 確定した候補IDを通知する |

初期選択は指定stateの有効IDを使い、なければ先頭、候補なしならnilです。初回は初期選択を通知し、その後は同じIDなら変更通知しません。初回callbackの延期は生成順序の規則に従います。

`reset_on`の変更では先頭に戻しますが、itemsの再生成だけでは戻しません。選択中のIDが候補から消えた場合は先頭、候補なしならnilにします。`select(nil)`による選択解除は、通常の再描画では維持します。行番号をIDとして扱わず、マウスとキーの確定は同じIDへ対応させます。

selection stateへ型は適合するが候補にないIDをsetした場合は、表示採用時に先頭/nilへ正規化します。`list:select(不存在ID)`のLuaエラーとは区別します。

selection stateと表示が新しい選択に同期してから`on_selection_changed`を呼びます。更新に対応する正常な表示の採用前やrender失敗中は、古い候補を確定させません。

`win:list(key)`で取得したハンドルを操作します。

| 呼び出し | 動作 |
|---|---|
| `list:select_next()` / `list:select_previous()` | 表示順で次／前へ移り、選択行を可視位置へスクロールする。未選択から次へは先頭、前へは末尾。`wrap_navigation = false`では端で選択を維持する。候補なしでは`false` |
| `list:select(id)` | 指定IDを選ぶ。`nil`では選択解除。有効な部品で存在しないIDを指定した場合はLuaエラー |
| `list:confirm()` | 現在の候補を確定する。選択なし・未同期では`false`を返し、後で自動実行するために保留しない |

部品削除でハンドルは失効し、同じkeyで再作成しても自動再接続しません。

### input

`ui.input`は`key`・`bind`必須、stringの`placeholder`の既定は空文字列です。

- `on_change(text)`は利用者による確定した編集で、bind先stateの更新後に呼ぶ。プログラムからの`set`では呼ばない。
- `on_submit()`は引数なし。IME変換確定のEnterでは呼ばない。
- `keys`はこのinputにフォーカスがあるときの追加割り当て。省略時は空。部品操作への接続はLuaで行う。

### source

`ui.source`は`key`・`source`必須です。初期版の`wrap`は`true`のみを受け付け、既定も`true`です。`highlight`は同じソースの範囲1個とし、nil・省略では強調しません。source/highlight/scroll_toのsource/rangeは所定のmmvハンドルだけを受け付け、通常tableから変換しません。文書の共有sourceハンドルを受け、生ソース全文を読み取り専用で表示します。Markdown整形やリンク先の読み込みは行いません。

同じsource・keyでは閲覧位置を維持し、source変更では位置を初期化します。highlightの変更だけではスクロールしません。リンク選択の候補なし・位置不明では、Luaがsource部品を外して以前の内容を残さないようにします。

`win:source(key)`で取得したハンドルを操作します。

| 呼び出し | 動作 |
|---|---|
| `source:scroll({ pages = 0.5 })` | source表示領域に対する有限数のページ量でスクロールする。正は下、負は上、0は無作用 |
| `source:scroll_to(range, { align = "center" })` | 同じsourceの範囲へ一度だけ移動する |

`scroll_to`のalignは`start`（範囲先頭を上端）、`center`（範囲中央を表示領域中央）、`end`（範囲末尾を下端）、`nearest`（見えていれば動かさず、必要な最小移動）です。options省略時とalignの既定は`nearest`です。表示領域より長い範囲への`nearest`では、範囲先頭を上端へ合わせます。文書端では可能範囲へ収めます。移動要求はレイアウト後に一度消費し、再描画・サイズ変更で再適用しません。`reveal`や`move`という別名の操作は設けません。

### button

`ui.button`は`key`・stringの`label`・functionの`on_click()`必須です。booleanの`enabled`の既定は`true`。`disabled`は重複して提供しません。

## 文書・リンク・ソース範囲

文書・ソースAPIはMarkdown専用です。`mmv.document.current()`は現在のMarkdownのdocumentハンドルを返し、文書がない場合やPDF表示中はnilを返します。PDFを空のMarkdown snapshotとして扱いません。標準のリンク選択はPDF表示中には起動しません。documentは取得時の読み取り専用snapshotです。本文置換後も取得時の内容を閲覧でき、暗黙に新しい本文へ切り替わりません。これは呼び出すたびに全文コピーするという意味ではありません。

| 呼び出し | 結果 |
|---|---|
| `doc:source()` | 共有sourceハンドル |
| `doc:links()` | 本文順のリンク候補配列 |

リンク候補は`id`、`display_name`、`destination`、`kind`、`target`、`source_range`を持ちます。display_name/destination/kindは文字列です。リンク解析に失敗した場合は空配列と偽らず、Luaエラーにします。

- IDはsnapshot内で安定したLua integerで、永続IDではない。
- `destination`は表示・照合用文字列。`kind`は`heading` / `markdown` / `markdown_heading` / `web`で、表示ラベルはLuaが組み立てる。
- `target`は解決済みの読み取り専用遷移先ハンドル。相対pathや同文書見出しを、後で別の現在文書を基準に再解決しない。
- 候補範囲・同名かつ同じ解決先の集約・最初の出現位置・HTMLリンク除外は[リンク候補の仕様](links-history.md#候補の範囲と表示)に従う。
- `source_range`は通常のtableではなく、`start` / `stop`を読める読み取り専用オブジェクト。文書内の0始まりUTF-8 byte・半開区間を表し、取得元sourceとの対応を内部に保持する。位置不明ならnilで、別sourceへの適用は拒否する。
- 古いsnapshotの閲覧はできるが、その本文が現在本文でなくなった後、targetによる新たな遷移は`stale_target`で拒否する。有効な時点で受け付け済みの遷移が本文を置き換えることは許可する。

読み取り専用ハンドルは、同じsnapshot内の同じ対象なら等しいものとします。範囲は元ソースとstart/stop、遷移先は解決済みの宛先で比較します。別snapshotは内容が同じでも別物とします。

### 本文置換の購読

```lua
local subscription = mmv.document.on_replaced({ owner = win }, function()
  win:close()
end)
subscription:cancel()
```

`owner`は必須で、callbackは引数なしです。閉じたownerへの登録は購読を作らず`false`を返します。有効なownerへの登録ではsubscriptionを返し、`subscription:cancel()`は解除できたら`true`、解除済みなら`false`を返します。

callbackは実際の新しい本文の採用とUI交換後に呼びます。MarkdownからPDFへの切り替えも通知し、callbackからの`current()`はnilになります。保持済みMarkdown snapshotはPDFへの切り替え後も閲覧できますが、古いtargetによる新要求は`stale_target`で拒否します。本文採用時点の購読を登録順に通知し、採用後に追加した購読は次回から対象とします。配送前に解除された購読やownerが終了した購読には通知しません。要求を受けただけ、本文を保持した失敗、古い未採用完了、スクロール、画像・Mermaid完了では通知しません。

ownerのwindow終了で購読を解除します。汎用float自体に文書依存を持たせず、Luaが終了処理を接続します。callbackが失敗しても、古いtargetを使えない検証はホスト側に残します。

## ファジーマッチ

`mmv.match.fuzzy(query, text)`は文字列2引数を受け、一致では`{ score = ..., ranges = { { start = ..., stop = ... }, ... } }`、不一致ではnilを返します。rangesはtext内の0始まりUTF-8 byte・半開区間です。空queryではtextによらず同じscoreと空rangesを返します。

Rustの既存照合計算を使い、同じqueryで対象を比較するscoreの大小を保証します。特定の最大値、空queryの具体的数値、版をまたぐ絶対値互換は保証しません。rangesは昇順・非重複とし、連続一致を結合して返します。照合対象・並べ方はLuaが決めます。リンク選択では表示名・宛先を別照合し、一致した側の最大scoreを用い、降順・同点本文順で並べます。不一致はscore値ではなくnilとして扱います。

候補配列を受ける旧`fuzzy(query, items)`や`fields`形式、`match.rank`は提供しません。ソース位置はMarkdown解析から取得し、照合結果から推測しません。

## 遷移と非同期結果

```lua
mmv.navigation.open(target, {
  owner = win,
  on_complete = function(result)
    -- Handle the result.
  end,
})
```

入力はdoc:links由来のtargetハンドルだけとし、任意URL/path文字列は受け付けません。optionsは任意、ownerはwindowまたはnil、on_completeはfunctionまたはnilです。閉ownerでも遷移を受け付けますが、結果callbackは登録しません。型・値・options等の契約違反は同期Luaエラーであり、遷移結果のcodeとは別です。

既存の文書操作・履歴を使います。openは戻り値を持たず、on_completeはopen実行中には呼ばず、戻った後に寿命条件の範囲で一度配送します。

| 結果 | callbackの値 |
|---|---|
| 成功 | `{ ok = true }` |
| 失敗 | `{ ok = false, code = "...", message = "..." }` |
| 警告付き成功 | `{ ok = true, warning = { code = "...", message = "..." } }` |

### 初期版の結果code

| code | 結果と意味 |
|---|---|
| `stale_target` | 失敗。現在本文でないsnapshotのtargetによる新要求。遷移せず現状保持 |
| `superseded` | 失敗。終端結果確定前に後続要求で結果/位置要求が無効化された。既に採用済みの変更を巻き戻さない |
| `file_not_found` | 失敗。対象ファイル不在。本文/履歴保持 |
| `access_denied` | 失敗。アクセス拒否・ロック。本文/履歴保持 |
| `invalid_utf8` | 失敗。対象文書のUTF-8不正。本文/履歴保持 |
| `read_failed` | 失敗。その他の同一性解決/読取失敗。本文/履歴保持 |
| `browser_open_failed` | 失敗。OSへのURL起動要求失敗。本文/履歴を変更しない |
| `navigation_failed` | 失敗。読込/採用前の、上記に分類できないホスト側の遷移失敗。本文/履歴保持 |
| `heading_not_found` | 警告付き成功。指定見出しがなく、先頭へ移動 |
| `position_unavailable` | 警告付き成功。本文採用または現在本文の確認は成功したが位置解析/適用ができない。本文/履歴を巻き戻さない |

未採用・無効化が確定した旧要求はsupersededを優先し、単なるadopted=falseでは判定しません。終端結果確定後は変更しません。採用後の位置失敗はok=falseではなく警告付き成功です。messageはアプリ生成の英語文言とし、名前・パス等は翻訳しません。

### 完了点と配送

- ローカル文書は本文・履歴の確定と要求した初期位置の適用、または位置適用不能の警告確定までを完了とする。同文書見出しも、読取不要でも見出し判定/位置適用後とする。見出し不在は先頭適用後の警告付き成功とする。
- 既存処理が同文書を読み込まず成功した場合はadopted=falseでも成功とする。すべての同文書リンクを新たに無読込にする変更ではない。
- WebはOSのURL起動要求が受付成功を返した時点で完了とし、ページ読み込み成功は保証しない。現行のcx.open_url呼出しだけをOS受付成功とみなさず、実装時に成否を観測できる接続を必要とする。
- 本文採用→置換通知→位置適用/結果完了の順とする。owner終了で未実行callbackを解除するが遷移は継続する。本文置換通知でownerが閉じて完了callbackが呼ばれない場合もある。window終了後も結果が必要ならownerを省略する。アプリ終了後の通知は保証しない。
- on_complete省略時は失敗/警告をホスト通知・ログへ送る。ただしsupersededは通常の画面通知を出さない。
- on_complete指定時は結果の画面通知をLuaへ委ね、重複する自動GUI通知はしない。必要なホストログは維持する。owner終了でcallbackが解除されても自動fallback通知を追加しない。callback自身の失敗は所属規則に従う。

読み込み失敗時の本文/履歴保持、見出し不在時の先頭表示等の利用者向け動作は[既存の遷移規則](links-history.md#リンク)に従います。

## 通知とエラー・失効

`mmv.notify(message, { level = "info" })`で通知します。levelは`info` / `warning` / `error`、既定は`info`です。渡されたmessageは翻訳せず既存の通知ポリシーで表示します。アプリ生成の既定文言は英語、Search/Commandの入力は保護し、新たな固定時間上限は設けません。GUIの言語と通知規則は[画面仕様](viewer-ui.md#表示言語と通知)に従います。

`on_error(err)`のerrは`{ message = "...", traceback = "...", phase = "event", key = nil }`の形式です。messageとtracebackは文字列、phaseは`"event"`または`"render"`です。関連部品を特定できる場合はkeyにその部品keyを入れ、それ以外はnilとします。

- `pcall` / `xpcall`で処理したエラーはwindowの`on_error`へ送らない。未処理のevent/renderエラーを`on_error`へ送り、Luaが継続・再試行・表示・終了を判断する。
- callbackエラーだけでホストが一律closeしない。`on_error`自体の失敗は再帰配送せず、ホストの通知・ログへ送る。
- `on_error`省略時、`on_close`の失敗、windowに属さないcallbackの未処理エラーはホストの通知・ログへ送る。初回render・検証失敗は生成順序の規則に従い、呼び出し元へのLuaエラーとし、生成途中のwindowから重複通知しない。
- callback途中までに完了したstate更新は巻き戻さない。後続render失敗では不完全なツリーを採用せず、直前の正常表示を保持する。入力と未同期のlistは確定させない。
- 閉じたwindow・失効した部品ハンドルへの操作は無作用で`false`とし、この寿命規則を優先する。有効な対象への不正引数、開いているwindowに存在しない部品IDなどはLuaエラーとする。
- 有効対象への不正な操作は作用前に拒否し、その操作を部分適用しない。以前のcallback等で完了済みのstate更新を巻き戻さない規則とは区別する。
- 解放・購読解除・終了後イベントの無効化は、Lua callbackが成功することを条件にしない。

## 受け入れ条件

- キー字句・正規化・重複・再登録と部品既定キーが契約どおりになり、IME優先を維持する。
- Tabは各window内を循環し、クリック/focusでz順を変えず、modal背後focusは非保留のfalseとなる。
- 型・UTF-8・必須/省略・配列・callback・ハンドルの入力規則を適用し、一般state.stringへ表示入力のUTF-8制約を広げない。
- 同期表示構築とイベント延期を区別し、所属・編集/選択通知順・終了後エラー配送を確認できる。
- 初期化フェーズと操作受理結果、作用前検証、所定の取得/生成値を維持する。
- 文書解析失敗を空候補と偽らず、fuzzyの範囲とscoreの公開保証を維持する。
- 全結果codeと警告を区別し、採用後の位置失敗を警告付き成功とする。同文書無読込成功をsupersededにしない。
- ローカルの位置適用/警告確定とWebのOS受付を完了点とし、owner・handler省略/指定に従って一度配送し、自動GUI通知を重複させない。
- 初期版のmode・単一キーの範囲に従い、IMEを優先しながら追加keysで同じ層の既定操作を上書きできる。callbackの戻り値やエラーで処理済みキーが背後へ流れない。
- 標準登録後に利用者分を一括で有効化し、失敗時は標準設定・標準キーを維持する。init実行中はstate操作ができ、window表示・遷移は行えない。キー登録APIの検証エラーをpcallで捕捉しても初期化全体が失敗する。
- フォーカス対象を指定して移動でき、開いているwindowの対象外部品への明示指定はLuaエラーとなる。閉じた対象の無作用falseと区別できる。
- render/render_itemは表示記述1個だけを返し、children/itemsは欠番のない配列となる。空配列とempty省略を受け付け、必須render_itemや戻り値の不正を拒否する。
- 寸法・比率・grow/gap/paddingの範囲と既定、比率の基準、alignの方向に従って配置し、過大なfloatは描画領域内に収める。
- textの範囲外・逆順・UTF-8文字途中を拒否し、空範囲は表示に影響せず、重複範囲の属性は後の明示指定を優先する。
- sourceのwrap/highlight/pagesの入力規則とscroll_toの整列・省略時nearestに従い、表示領域より長い範囲と文書端も扱える。
- 初回render・検証・部品取得・初期callback・レイアウト後スクロールの順序を確認できる。
- 同じkeyと型で状態を保持し、部品削除後の旧ハンドルを同じkeyの新部品へ再接続しない。
- stateの許容値を受け付け、不正な型・値を拒否する。初期値・set・getのtableコピーとreadonlyハンドル共有により、取得tableの編集がset前にstateへ影響せず、文書全文もコピーしない。
- tableキーとコピー内の共有参照、Lua integerのID、snapshotに属するreadonlyハンドルの比較、返却tableの独立性が契約どおりとなる。
- 同一stateへの複数の双方向bindを許可せず、bind型不一致のsetは元の値を保って拒否する。on_changeは利用者の確定した編集でのみ呼ばれ、部品削除・無効化時には所定のフォーカス移動を行う。
- 操作の受理trueと個別のfalseを区別し、未同期confirmを後で自動実行しない。取得の型不一致はLuaエラー、navigation.openの結果は戻り値でなくcallbackとなる。
- 同値setでも更新を通知し、同一callback内の反映をまとめる。reset_onは許容する単純値の比較で変更を判定する。
- 候補消失・選択解除・未知ID・未選択からの次/前・非wrap端の選択規則を維持し、selection stateと表示の同期後に変更を通知する。初期選択・通知・確定IDが一致し、正常表示採用前やrender失敗中の古い候補を確定しない。
- snapshot・source_range・targetの対応を保ち、別sourceの範囲と古いtargetを拒否する。
- 必須ownerと購読の寿命に従い、本文採用時点の購読を登録順に引数なしで通知する。採用後追加・配送前解除を区別し、閉ownerへの登録と反復cancelの結果を確認できる。結果の一度通知・callback解除と遷移継続を区別できる。
- on_close/on_errorの値が所定の形式となり、handler省略・on_close失敗・window外callbackの未処理エラーがホストへ届く。初回生成失敗は呼び出し元へのLuaエラーとなり、生成途中のwindowから重複通知されない。
- 文字列2引数の照合結果から、空query・不一致・一致を混同せずリンク候補を並べられる。
- エラー後も正常表示・完了済みstateを保ち、終了処理や古い対象の検証を継続できる。
