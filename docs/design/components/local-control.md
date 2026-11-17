# ローカル通信・起動制御

[設計全体像に戻る](../index.md)

## 責務と対象範囲

[OS抽象化と共通契約](../../requirements/features/os-abstraction.md)、[外部操作](../../requirements/features/external-control.md)、[通信設定](../../requirements/features/display-settings.md#通信設定)を実現する。Windows Named Pipe・名前付きMutexを、共通のTCP通信とファイルロックへ置き換える。

文書操作の採否・GUI配送・入力状態・成功応答の意味は[CLI・単一起動・外部操作](external-control.md)に従う。本書を通信・起動・配置・安全性の正本とし、同文書の移行前のPipe固有設計を新方式へ適用しない。LinuxのGUI・フォント・IME・X11／Wayland・Mermaid・デスクトップ統合・配布は対象外とする。Mermaidのscratch配置は変更しない。PDFは既存のpure Rustのhayroを維持し、Pdfiumへ変更しない。

## 構成と責務

| 対象 | 責務と境界 |
|---|---|
| `config`・Lua初期化 | `ipc.port`を含む候補設定と登録を検証し、成功時だけ一括採用する |
| `infra`の配置解決 | ユーザー基準から設定・ログ・instanceの絶対パスを決める |
| `infra/ipc` | IPv4 loopbackへのTCP接続・受付、フレーム検証、期限、要求ID、切断と応答先の寿命を管理する |
| `infra/instance` | live／startupの共通状態遷移と、OS別の安全なファイル操作・GUI起動を組み合わせる |
| `main` | 引数・help/version/usage処理後に設定を読み、1要求を送って終了する |
| `runtime` | GUI設定読込後にliveを保持し、bind・ウィンドウ生成・文書配送・終了を接続する |

OS差のためのアダプターはネイティブパス変換、安全な保存先・ファイル作成、非継承ハンドル、プロセス起動に限定する。TCP・プロトコル・起動判定をOSごとに複製しない。既存のTokioを共通依存へ移し、current-thread runtimeと必要なfeatureだけを使う。Linuxの安全なファイル操作は`libc = 0.2.191`をLinux向けの直接依存として使用し、対象ABIに一致した定数・関数・型を参照する。既存lock版を固定し、他の依存追加・更新は行わない。

## 設定読込とLuaの寿命

CLI・GUIとも、設定評価を接続・live確認・startup取得・bindより前に完了する。help/version/usageは設定や通信資源を必要としない入口として維持する。

- `mmv.setup({ ipc = { port = 43210 } })`を受け付ける。省略時は43210、1〜65535の整数だけを許可する。0、範囲外、非整数、不正型、未知キーは設定エラーとする。
- 同じLua初期化API・phase制約を両入口で使う。keymap・state・純粋な表示記述作成は登録／評価できるが、init中の文書参照・購読、notify、window表示・遷移などは[Lua公開API](../../requirements/features/lua-api.md#初期化と設定の有効化)に従って禁止する。
- CLIは有効化された設定から通信先を取り出すだけとし、キーcallbackやGUI host出力を実行しない。GUIは成功したVMとclosureを保持し、二度目のユーザーinit評価を行わない。
- 読込・Lua実行・検証のどこで失敗しても候補VMを捨て、設定全体と登録全体を組み込みbaselineへ戻す。`ipc.port`だけを救済しない。keymap等の登録検証失敗は`pcall`で捕捉されても初期化全体を失敗させる既存規則を維持する。
- init.luaを書き換えない。GUIの`Unable to load configuration`と既存ログの通知規則を維持する。CLIの評価は既存GUIへ設定を再適用しない。
- 設定変更はGUI再起動で反映する。CLIとGUIが別の設定時点を読む可能性はある。生存中のGUIのポートと異なれば接続期限内に失敗し、別GUIや別ポートへ自動迂回しない。
- bind失敗、ロックのI/O・権限・保護検証失敗は実行時エラーとし、設定fallbackで43210へ再bindしない。

現行`LuaRuntime::initialize_with`はbaselineと候補VMを分ける。`load_config`とViewer側の標準キー導入の境界も確認し、失敗した候補のclosure・state・登録・host出力を新VMへ移さない。CLIにGUIアダプターを装着しないことと、init時に使えるAPIの差を作らないことを両立させる。

## 通信・フレーム契約

### 接続先と信頼境界

サーバーは`127.0.0.1:<port>`だけへbindし、CLIも同じIPv4アドレスだけへ接続する。ホスト名解決・IPv6・`0.0.0.0`・HTTP・代替ポート・認証は追加しない。bind競合は失敗である。

CLIはliveロックを生存判定の正本とし、live不在のTCPサーバーを既存GUIとは扱わない。ただし、ロックの所有者検証はTCP相手の所有者検証ではない。live生存中や起動後の接続先について、別ユーザーのクライアント、ポートを先取りした偽サーバーを排除しない。プロトコル版・フレーム・要求IDの検査も認証にはならない。ユーザーごとに異なるポートを使う運用上の回避と、この信頼境界を明記する。接続後に不正な応答を受けても、新しいGUIの起動や要求再送で回避しない。

### バイナリ形式

1接続につき要求1個・応答1個とする。通信版は2から3へ更新し、旧版・未知版への互換処理は持たない。

| 要素 | 表現と制約 |
|---|---|
| フレーム | payload長のu32 LE＋payload。長さは1〜65536 bytes。確保前に上限検証する |
| 共通prefix | u16 LEの版3、u8のkind（要求1／応答2）、u32 LE長＋要求IDのUTF-8 bytes |
| 要求ID | 空不可、最大256 bytes。応答と完全一致させ、GUI文書世代とは別に扱う |
| 操作 | u8 opcode。Open=0、Scroll Down/Up/HalfDown/HalfUp/PageDown/PageUp=1〜6、Top=7、Bottom=8、Back=9、Forward=10 |
| Open path | u8のencoding discriminator＋u32 LEのbyte長＋native bytes。1=Windows UTF-16LE、2=Linuxのバイト列 |
| Open line | pathの後に有無のu8（0／1）。1の場合だけ正のu64 LE。0や未知flagは拒否する |
| 応答 | prefix後にu8 outcome（成功0／失敗1）。失敗時だけu32 LE長＋UTF-8の理由 |

既存の各可変長fieldの上限60000 bytesを維持する。切詰め、trailing bytes、未知kind/opcode/encoding、NULを含むpath、相対path、対応OSでないpath表現、UTF-16LEの奇数byte長、ID・理由の不正UTF-8を拒否する。UTF-16の単独surrogateは有効なWindows path単位として保持し、Linux pathにはUTF-8検証を適用しない。Windowsでencoding 2、Linuxでencoding 1は暗黙変換せず拒否する。

CLIが自身のcwdを基準にpathを絶対化し、GUIが本文を読み込む。本文を通信へ載せない。Openの成功は必要な読込・PDF解析等の結果を待つが、描画・Mermaid・画像・指定行の可視化完了は待たない。

### 切断・期限・資源

- 接続5秒、起動30秒、応答30秒の各段階で絶対期限を1回だけ決める。接続段階は最初のlive検査前、起動段階は未起動Openのstartup取得前に開始し、startup待機・live再確認・spawn・接続で同じ起動期限を共有する。接続再試行や部分フレームの進捗で期限を延長しない。読み込み不要の操作も応答期限を使う。
- TCP接続1試行は100msで打ち切り、未完了の接続futureを破棄する。段階期限の残時間が短ければ段階期限を優先する。専用markerを持つ試行timeoutだけを既存の再試行可能な接続エラーへ加え、段階期限やOS由来の通常の`TimedOut`、権限等の非再試行エラーは再試行しない。再試行は接続確立前だけであり、要求送信の再試行ではない。
- 受信後・GUIキュー投入前に、TCPの非待機readで既に観測できる切断／余分なデータを検査する。後続の切断は投入と競合し得る。受理済み操作を取消しとは扱わない。
- GUI応答待ちと、read readinessによるEOF／余分なデータの検査を同時に待つ。切断・期限で接続の生存状態をfalseにしてNotifyし、GUIの応答先を解放する。送信済みの文書操作自体は継続できる。
- 部分read/writeを取り消した接続は破棄し、再利用・再送しない。CLI timeoutはGUIを終了しない。失った応答から操作未実行と推論しない。
- TCPではPipeのMio内部書込バッファを保全するための1 byte receipt、receipt後の切断待ち、`PeekNamedPipe`、`CancelIoEx`を引き継がない。CLIは完全な版3応答をdecodeしIDを検証した時点で結果を受理する。サーバーは期限付きwrite完了後に接続を閉じる。TCP write完了は相手の受理証明ではないため、応答喪失時の再送禁止は維持する。
- 専用IPCスレッド・current-thread runtime、接続JoinSet上限128、GUI受信キュー上限128、try_sendで満杯を拒否する構成を維持する。外部IDと内部ConnectionIdを区別する。
- 停止は受付停止→接続future取消・JoinSet回収→runtime終了→IPCスレッドjoin。GUIスレッドでjoinを待たない。bindはlistener準備完了後に成功を返し、初期化失敗は呼出元へ返す。

## 配置と安全なファイル操作

| 用途 | Windows | Linux |
|---|---|---|
| 設定 | `%LOCALAPPDATA%\mmv\init.lua` | `<home>/.config/mmv/init.lua` |
| ログ | `%LOCALAPPDATA%\my-markdown-viewer\logs\viewer.log` | `<home>/.local/state/mmv/viewer.log` |
| ロック | `%LOCALAPPDATA%\mmv\instance/live.lock`・`startup.lock` | `<home>/.local/state/mmv/instance/live.lock`・`startup.lock` |

Windowsは空・相対・未設定のLOCALAPPDATAを拒否する。Linuxはユーザーhomeの絶対パスを解決し、`XDG_CONFIG_HOME`・`XDG_STATE_HOME`・`XDG_RUNTIME_DIR`を参照しない。解決失敗をcwdで代替しない。設定読込の失敗はbaselineへ復帰できても、安全なinstance保存先が得られなければ通信・起動を続けない。Mermaidのscratchは別契約のまま変更しない。

安全性の境界はファイルロック操作だけではない。以下をOSアダプターの契約とする。

- Linuxは現在ユーザー所有のinstanceディレクトリ0700、通常ファイル0600を使用する。既存対象も所有者・種別・mode・hard link数を検査する。緩い権限の既存対象を黙って使ったり、安全性確認前にchmod／truncateしない。
- homeからinstanceまでの経路について、リンク・非ディレクトリ・他ユーザー所有・他者が書換可能な経路を拒否する。0700を要求する範囲はinstanceディレクトリであり、祖先すべてを0700へ変更しない。
- Linuxは検証済み親ディレクトリを基点とする相対open、no-follow、排他的新規作成、作成後のhandleメタデータ照合を使い、パス事前検査だけで安全としない。ファイルdescriptorはclose-on-execにする。
- Windowsは現在ユーザーの明示owner・保護DACL・非継承handleでinstanceと通常ロックファイルを新規作成し、既存対象でもowner・ACL・種別を検査する。reparse point／junctionを経路と対象で拒否し、検証した経路handleを保持してrename/deleteによる差し替えを防ぐ。作成競合では既存対象を安全に開き直し検証する。
- 両OSで検証したファイルhandle自身にロックし、シンボリックリンクの追跡、存在確認とopenの間の差し替え、子へのhandle継承で保護を失わない。安全性を検証できない場合はfail closedとし、未起動とは扱わない。
- 通常終了も異常終了もOSがhandleを閉じることで排他を解放する。ロックファイルは削除しない。PIDやファイルの有無を生存判定に使わない。

ユーザー所有のローカルディスクを通常利用とする。ネットワーク／共有homeではホスト間ロック実装とloopbackの範囲が一致しないため、複数ホストにまたがる単一起動・GUI接続は保証しない。ローカルセッション／ポートの違いと、この制約を混同しない。

## ロックの状態と起動手順

Rust 1.89以降の`std::fs::File::try_lock()`を使う。`WouldBlock`相当の競合を、その他のI/O・権限エラーと型／結果で分ける。通常のmutexやファイル存在確認で代替しない。

| ロック | 所有者と寿命 | 不変条件 |
|---|---|---|
| live | GUI自身。GUI初期化からウィンドウ・IPC終了後まで保持 | ユーザーにつき1 GUI。session／portをキーに含めない |
| startup | Openの起動調停をするCLIだけ | GUI生存ロックの代わりにせず、起動調停後は解放 |

1. 設定読込後、接続段階の5秒の絶対期限を設定し、TCP接続より先にliveを検査する。取得できれば確認用handleを解放して未起動と判断し、競合なら生存と判断する。その他のI/O・権限・保護検証エラーは直ちに失敗する。
2. 生存中なら確定したportへ同じ接続期限内で接続する。準備完了待ちも期限を共有し、接続不能やポート不一致でも別GUIを起動しない。途中でGUIが終了しても起動判定へ戻らない。
3. 未起動かつOpen以外ならTCP接続もGUI起動も行わず、`NotConnected`で失敗する。live不在のTCP待受を既存GUIとして採用しない。
4. 未起動Openは起動段階の30秒の絶対期限を1回設定し、startupの取得を同じ期限内で待つ。ロック競合だけを待機対象とし、その他のエラーは未起動と解釈せず失敗する。
5. startup取得後はTCPを試さずliveを再検査する。待機中に他のCLIが起動したGUIが生存していれば追加起動せず、未起動の場合だけ期限内に現在の実行ファイルを内部GUI引数で起動する。live検査のエラーでは起動しない。
6. GUIは自身で設定を読み、liveを取得してからbindする。live競合なら追加GUIとしては動作しない。bind競合・安全性・初期化失敗はエラーとし、代替portを選ばない。
7. CLIは既存または起動したGUIへ同じ起動期限内に接続し、startupを解放する。失敗経路でもstartupを解放する。生存検査前とstartup取得後の先行TCP試行は行わない。接続成功後に要求は一度だけ送り、別に設定した応答30秒の期限で結果を待つ。以後は起動判定へ戻らず、接続・応答失敗でも要求を再送しない。

起動する実行ファイルはcurrent executableに限定し、任意シェルコマンドを組み立てない。Windowsの非アクティブ起動を保持し、子へlock handleを継承させない。Linuxの実GUIでフォーカスを奪わないことは、プロセス起動だけでは証明しない。期限切れではCLIだけが失敗し、起動済みGUIは強制終了しない。

## 現行実装からの差分

| 箇所 | 差分 |
|---|---|
| `src/main.rs` | 起動調停前にLuaを評価し、確定portと配置を渡す |
| `src/runtime.rs` | live取得・bindより前へ設定読込を移す。成功VMをそのままViewerへ渡す |
| `src/config.rs`・`src/config/lua.rs` | ipc設定と厳密な整数検証を追加し、既存の全体rollbackへ含める |
| `src/infra/config.rs`・logging | OS別配置の解決を分離し、失敗時にcwdを使わない |
| `src/infra/ipc/` | PipeをTCPへ置換し、版3・encoding discriminator・切断監視・期限を実装する |
| `src/infra/instance.rs`・security | 名前付きMutexを安全なファイルロックへ置換し、必要なOS別処理だけ分離する |
| `Cargo.toml` | 既存Tokioの配置を共通化し、Rustの最小版と必要featureを整える |

## 検証観点

- portの境界・不正型、GUI専用APIのinit禁止、登録途中の失敗とpcall捕捉でも完全rollbackすることを確認する。
- 配置解決に対するXDG無視・home不明・相対基準拒否と、設定読込が通信・生存判定へ先行する順序を確認する。
- 全操作・要求ID・UTF-16単独surrogate／Linux非UTF-8、旧版・不正・過大・途切れたフレームを検証する。
- 隔離したTCPと別プロセスロックで同時Open、live生存＋接続不能、非Open、異常終了再取得、残存ファイル、I/O・権限エラー、継承、リンク差し替え、port競合・応答喪失・timeout・再送禁止を確認する。
- テストは専用ディレクトリとtest-onlyのephemeral portを使用し、通常ユーザーの43210やinstanceロックを使用しない。本番設定でport 0を許可する意味ではない。
- 既存のGUI境界回帰試験で入力モード・文字列・Open成功条件を確認し、実GUIのfocus確認とは区別する。Windowsのcargo検証とLinux実機・GUIの検証範囲を分ける。
