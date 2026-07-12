# P/ECE Resource Reservation Model for KotoOS

## 0. Scope

この文書は、P/ECEのCPU/タイマ/DMA/割り込み/I/Oなどの限られた資源を、OS側がどのように予約し、アプリにはどの抽象APIとして見せていたかを整理する。目的はKotoOS/PicoCalcのリソース所有権、hostcall境界、HAL境界を設計することであり、P/ECE互換APIや実装移植は扱わない。

P/ECEの関数定義、構造体定義、テーブル、回路図、素材は複製しない。API名とファイルパスは根拠参照としてのみ記載する。

主な根拠ファイル:

- `docs/PIECE ハードウエア割り込み.htm`
- `docs/PIECE ポート解説.htm`
- `docs/API/pceTimer*.html`
- `docs/API/pceLCD*.html`
- `docs/API/pceWave*.html`
- `docs/API/pcePad*.html`
- `docs/API/pceUSBCOM*.html`
- `docs/API/pcePower*.html`
- `docs/API/pceIR*.html`
- `sysdev/pcekn/mainloop.c`
- `sysdev/pcekn/runapp.c`
- `sysdev/pcekn/pcekn.c`
- `sysdev/pcekn/timer.c`
- `sysdev/pcekn/lcd.c`
- `sysdev/pcekn/snd.c`
- `sysdev/pcekn/pad.c`
- `sysdev/pcekn/usbcom.c`
- `sysdev/pcekn/file.c`
- `sysdev/pcekn/powerman.c`
- `sysdev/pcekn/hard.c`

## 1. システム予約リソース一覧

| リソース種別 | 使われる用途 | 関係するハードウェア機能 | 関係するOS/低レイヤモジュール | アプリから直接触れるか | アプリに見せているAPIカテゴリ | 競合時に起きそうな問題 | 根拠ファイル |
|---|---|---|---|---|---|---|---|
| 1ms基準タイマ | OS時刻、アプリ周期、タイマコールバック、簡易スケジューリング | 16bitタイマ、NMI/タイマ割り込み | `timer.c`, `runapp.c` | 直接設定は不可。カウント取得とコールバック登録のみ | Timer, App period | コールバック過多で割り込み処理が伸び、入力更新、音声、USB、アプリ周期が乱れる | `sysdev/pcekn/timer.c`, `sysdev/pcekn/runapp.c`, `docs/API/pceTimerSetCallback.html` |
| Timer callback枠 | アプリまたはシステムの遅延/周期処理 | 1msタイマ上のソフトタイマ | `timer.c`, `powerman.c` | 限定的に可。ただし割り込み文脈 | Timer | 長時間処理や危険なAPI呼び出しで割り込み遅延、スタック破壊、再入不整合 | `sysdev/pcekn/timer.c`, `sysdev/pcekn/powerman.c`, `docs/API/pceTimerSetCallback.html` |
| LCD物理転送 | VRAM相当バッファからLCDコントローラへ反映 | シリアルI/F ch3、HSDMA ch0、LCD制御ポート | `lcd.c`, `hard.c` | 基本不可。転送APIのみ。Direct転送は低レベル寄りで通常用途非推奨 | LCD draw/flush/buffer | DMA ch0やシリアルch3の横取りで画面破損、転送停止、LCD仕様依存の露出 | `sysdev/pcekn/lcd.c`, `sysdev/pcekn/hard.c`, `docs/API/pceLCDTrans.html`, `docs/API/pceLCDTransDirect.html`, `docs/PIECE ハードウエア割り込み.htm`, `docs/PIECE ポート解説.htm` |
| LCD論理バッファ | アプリ描画面、システムメニューの退避/復帰 | SRAM上の仮想画面 | `lcd.c`, `draw.c`, `runapp.c`, `biapp.c` | バッファアドレス設定は可。物理LCDは不可 | LCD buffer/draw | システムメニューや電池表示との上書き、未flush、アラインメント違反 | `sysdev/pcekn/lcd.c`, `sysdev/pcekn/runapp.c`, `docs/API/pceLCDSetBuffer.html` |
| 音声PWM | PCM/ADPCM再生、スピーカー/ヘッドホン出力 | PWM出力、16bitタイマ1、HSDMA ch1、DMA割り込み | `snd.c`, `hard.c`, `powerman.c` | 不可。WAVEキューと音量APIのみ | Wave/audio | DMA ch1やPWMタイマ競合でノイズ、無音、割り込み嵐、低消費電力復帰失敗 | `sysdev/pcekn/snd.c`, `sysdev/pcekn/powerman.c`, `docs/API/pceWaveDataOut.html`, `docs/API/pceWaveStop.html`, `docs/PIECE ハードウエア割り込み.htm`, `docs/PIECE ポート解説.htm` |
| 音声ミキサ作業領域 | 複数chのWAVE合成、出力バッファ二重化 | SRAM、DMA供給バッファ | `snd.c` | WAVEデータは渡すが、ミキサ作業領域は不可 | Wave/audio | 再生中データ破棄、キュー破壊、終了コールバック再入 | `sysdev/pcekn/snd.c`, `docs/API/pceWaveDataOut.html`, `docs/API/pceWaveCheckBuffs.html` |
| USBコントローラ | PC接続、制御転送、USBCOM、モード切替 | PDIUSBD12、USB INT_N、USB 6MHz、外部I/O空間 | `mainloop.c`, `usbcom.c`, `d12ci.c`, `hard.c` | 低レイヤ不可。USBCOM/USB接続APIのみ | USB, USBCOM | endpoint状態破壊、PC切断、給電状態誤判定、割り込み再入 | `sysdev/pcekn/mainloop.c`, `sysdev/pcekn/usbcom.c`, `sysdev/pcekn/hard.c`, `docs/API/pceUSBCOMSetup.html`, `docs/API/pceUSBCOMStartRx.html`, `docs/API/pceUSBCOMStartTx.html`, `docs/PIECE ポート解説.htm` |
| USB割り込み | D12イベント処理、endpoint完了、bus reset/suspend | K51 INT_N、割り込みレベル、D12イベントレジスタ | `mainloop.c`, `hard.c` | 不可 | USB/USBCOM状態API | 取りこぼしで転送停止、PC側プロトコル不整合、suspend処理遅延 | `sysdev/pcekn/mainloop.c`, `sysdev/pcekn/hard.c`, `docs/PIECE ハードウエア割り込み.htm`, `docs/PIECE ポート解説.htm` |
| 入力ポート | ボタン状態、トリガ、リピート | GPIO/Kポート | `pad.c`, `runapp.c`, `powerman.c` | 直接読みに近いAPIはあるが、通常は周期更新済み状態を読む | Pad/input | アプリ側独自scanでトリガ/リピートとシステム操作がずれる | `sysdev/pcekn/pad.c`, `sysdev/pcekn/runapp.c`, `docs/API/pcePadGet.html`, `docs/API/pcePadGetDirect.html`, `docs/API/pcePadGetProc.html` |
| 電源/standby制御 | USB/電池判定、電池表示、standby/wakeup | 電源GPIO、ADC、キー割り込み、LCD sleep、音声amp | `powerman.c`, `lcd.c`, `snd.c`, `hard.c` | 状態取得と明示standby要求のみ | Power | USB給電中standby、LCD/音声復帰漏れ、キー割り込み競合、電池消耗 | `sysdev/pcekn/powerman.c`, `docs/API/pcePowerGetStatus.html`, `docs/API/pcePowerEnterStandby.html`, `docs/API/pcePowerForceBatt.html` |
| ADC | 電池電圧/給電状態の取得 | ADC ch、ADC割り込み | `powerman.c` | 不可。Power statusのみ | Power | 電源監視値の破壊、割り込み競合、誤standby判断 | `sysdev/pcekn/powerman.c`, `docs/API/pcePowerGetStatus.html` |
| Flash/PFFS | ファイル検索、読み書き、作成、削除 | Flash erase/write、FAT/Directory相当管理 | `file.c` | セクタAPIは可だが物理Flash管理は隠蔽 | File/storage | erase単位競合、ディレクトリ/FAT破壊、電源断時破損 | `sysdev/pcekn/file.c`, `docs/API/pceFileReadSct.html`, `docs/API/pceFileWriteSct.html` |
| 割り込み/KSベクタ | trap/カーネルサービス登録、低レイヤ差し替え | trap table、kernel service vector | `pcekn.c`, 各Init関数 | APIはあるが通常アプリ向けというより低レイヤ/開発者向け | Vector/system | OS所有ISRの上書き、API呼び出し先破壊、復帰不能 | `sysdev/pcekn/pcekn.c`, `sysdev/pcekn/timer.c`, `sysdev/pcekn/lcd.c`, `sysdev/pcekn/snd.c` |
| CPU速度/バスwait | 省電力、LCD/USBアクセスwait調整 | CPU clock、外部バスwait、LCDタイマ設定 | `hard.c` | APIあり。ただしLCD転送保証に制約 | CPU/system | 低速設定でLCD転送不安定、USBサイクルタイム違反 | `sysdev/pcekn/hard.c`, `docs/API/pceCPUSetSpeed.html` |
| IR送受信 | P/ECE間赤外線通信 | IR Tx/Rxポート、タイマ/割り込みと推測 | `irsub.c`, API docs, `hard.c` | 専用APIのみ。送受信は排他的 | IR | 同時送受信不可、callback内API制限、長時間通信でstandby阻害 | `docs/API/pceIRStartRxEx.html`, `docs/API/pceIRStartTxEx.html`, `docs/API/pceIRStop.html`, `sysdev/pcekn/hard.c` |

## 2. タイマ/DMA/割り込みの責務分担

### LCD転送

P/ECEはLCDの物理転送をアプリから隠し、アプリには「仮想画面バッファへ描く」「明示的に転送する」という境界を見せている。低レイヤでは、LCD制御ポート、シリアルI/F ch3、HSDMA ch0、LCD用タイマ/トリガ設定を組み合わせている。

責務分担:

- アプリ: LCDバッファの内容を作る。必要なタイミングでLCD転送APIを呼ぶ。
- LCDモジュール: 画面形式変換、向き、輝度、電池表示の重ね込み、転送範囲管理を担当する。
- HAL/低レイヤ: シリアル転送、HSDMA ch0、LCD CS/RS/RESET、転送待ちを担当する。
- 割り込み/DMA: 資料上は高速DMA ch0がLCD転送に割り当てられている。ただし実装はDMA完了割り込みを常用せず、状態待ちで同期している箇所がある。

競合判断: LCD物理転送をアプリに触らせると、DMA ch0、シリアルch3、LCDポート、CPU速度設定が同時に壊れる。KotoOSではdisplay flush taskの内部資源にするべき。

根拠: `sysdev/pcekn/lcd.c`, `sysdev/pcekn/hard.c`, `docs/API/pceLCDSetBuffer.html`, `docs/API/pceLCDTrans.html`, `docs/API/pceLCDTransDirect.html`, `docs/PIECE ハードウエア割り込み.htm`, `docs/PIECE ポート解説.htm`

### 音声PWM/音声バッファ

音声はアプリに「WAVEデータを出力chへ投入する」抽象を見せ、PWM値の生成、DMA、タイマ、アンプ電源、低消費電力停止はシステムが持つ。実装では合成用バッファ、最終出力バッファ、再生キューを持ち、DMA割り込みで次の出力ブロックを供給する。

責務分担:

- アプリ: 再生データ、チャンネル、連続再生、終了通知を指定する。
- 音声モジュール: 複数chの合成、音量、キュー、終了コールバック、再生停止を担当する。
- HAL/低レイヤ: PWMタイマ、HSDMA ch1、DMA割り込み、アンプ電源を担当する。

競合判断: PWMやDMA ch1を別用途に使うと音声が破綻する。アプリにはaudio buffer/mixer APIだけを見せるべき。

根拠: `sysdev/pcekn/snd.c`, `sysdev/pcekn/powerman.c`, `docs/API/pceWaveDataOut.html`, `docs/API/pceWaveAbort.html`, `docs/API/pceWaveStop.html`, `docs/PIECE ハードウエア割り込み.htm`, `docs/PIECE ポート解説.htm`

### 1ms tick

P/ECEの1ms tickはOS全体の時間基準である。カウント取得、アプリ周期待ち、Timer callback、context switch hook、standby判定などがこの上に乗る。Timer APIのcallbackはハードウェアタイマ割り込み文脈から呼ばれるため、呼べるAPIや処理時間に制限がある。

責務分担:

- タイマISR: tick処理、ソフトタイマ処理、必要ならcontext switch hookを呼ぶ。
- アプリループ: `ClockTicks` を見てperiodic_procの周期を制御する。
- アプリ: カウント取得または限定的なcallback登録を行う。

競合判断: KotoOSでは1ms相当のsystem tick/monotonic clockをカーネル所有にし、アプリtimerはメッセージ/イベントとして配送する方がよい。割り込み内でアプリコードを直接呼ぶ設計は、PicoCalcでは避けるべき。

根拠: `sysdev/pcekn/timer.c`, `sysdev/pcekn/runapp.c`, `docs/API/pceTimerGetCount.html`, `docs/API/pceTimerSetCallback.html`, `docs/API/pceTimerSetContextSwitcher.html`

### 入力更新

P/ECEはハードウェア直接読み取りAPIを持つ一方、通常のアプリには「システムが周期処理前に更新した状態」を読むモデルを推奨している。`runapp.c` の周期ループでは、USB処理後にPad更新を行い、その結果でシステムメニュー、終了要求、standby判定、アプリ周期処理が動く。

責務分担:

- 入力低レイヤ: GPIO/Kポートから現在値を読む。
- Padモジュール: 前回値との差分、トリガ、リピートを作る。
- アプリループ: アプリのperiodic_proc前に入力状態を更新する。
- アプリ: 更新済み状態を読む。

競合判断: アプリが独自にscan周期を持つと、OSショートカットやrepeatとズレる。KotoOSではinput scan/update/repeatをOS所有にし、アプリには状態スナップショット/イベントを渡す。

根拠: `sysdev/pcekn/pad.c`, `sysdev/pcekn/runapp.c`, `docs/API/pcePadGet.html`, `docs/API/pcePadGetDirect.html`, `docs/API/pcePadGetProc.html`

### USB通信

USBは低レイヤD12イベントを割り込みで拾い、メイン側の `ProcUSB` がフラグ処理と制御転送処理を進める。USBCOMはさらに上位で、アプリにはsetup/start rx/start tx/get stat/stopというブロック転送APIを見せる。

責務分担:

- USB ISR: bus reset、endpoint完了、EOT、suspendなどのイベントを拾って状態を更新する。
- USB main処理: setup packet、DMA setup、suspend changeを処理する。
- USBCOM: アプリ向けの一ブロック送受信状態機械を提供する。
- アプリ: USBCOMの状態を見て送受信を開始/完了確認する。

競合判断: endpoint、D12 DMA設定、USBクロック、給電切替は一体で管理される。KotoOSではmonitor/run/install transportをOSまたはsystem app所有にし、通常アプリのUSB直接制御は不可にする。

根拠: `sysdev/pcekn/mainloop.c`, `sysdev/pcekn/usbcom.c`, `sysdev/pcekn/hard.c`, `docs/API/pceUSBCOMSetup.html`, `docs/API/pceUSBCOMGetStat.html`, `docs/API/pceUSBCOMStartRx.html`, `docs/API/pceUSBCOMStartTx.html`, `docs/API/pceUSBCOMStop.html`, `docs/PIECE ポート解説.htm`

### IR

IR APIは送信/受信/停止/状態取得を提供し、API文書では送受信API同士が排他的で、完了まで他の赤外線送受信が機能しないと説明されている。callbackを使う場合もTimer callback相当の注意が求められる。

責務分担:

- IR低レイヤ: IR電源、Tx/Rxポート、タイミング生成/計測を担当する。
- IR API: 一回の送信または受信セッションを所有する。
- アプリ: セッション開始、状態確認、停止、終了callbackを扱う。

推測: 指定調査対象に `irsub.c` は含まれていないが、`hard.c` のポート初期化とAPI文書から、IRはGPIOとタイミング資源を使う低レイヤ排他デバイスと見なすのが妥当である。

根拠: `docs/API/pceIRStartRxEx.html`, `docs/API/pceIRStartTxEx.html`, `docs/API/pceIRStartRxPulse.html`, `docs/API/pceIRStartTxPulse.html`, `docs/API/pceIRStop.html`, `sysdev/pcekn/hard.c`

### standby/wakeup

standbyは単なるAPI呼び出しではなく、USB接続/給電判定、LCD sleep、音声アンプ停止、IR電源停止、割り込みマスク退避、キー割り込み設定、低速タイマ設定、sleep命令、復帰後の状態復元を含むシステム遷移である。

責務分担:

- Powerモジュール: standby可否判定、電源状態取得、ADC更新、電池表示を担当する。
- LCD/音声モジュール: sleep/restartやDMA割り込み復帰に協力する。
- アプリ: standby通知に応答し、必要ならアクティブ応答で遷移を猶予する。

競合判断: standbyは複数デバイスの所有権を一時的にOSへ集約する処理であり、通常アプリに低レイヤ手順を公開してはいけない。

根拠: `sysdev/pcekn/powerman.c`, `sysdev/pcekn/lcd.c`, `sysdev/pcekn/snd.c`, `sysdev/pcekn/runapp.c`, `docs/API/pcePowerEnterStandby.html`, `docs/API/pcePowerGetStatus.html`

## 3. メインループと周期処理

P/ECEのアプリ実行は、プリエンプティブな一般OSというより、1ms tickを基準にした協調的な周期ループである。責務の流れは次のように整理できる。

1. 起動時にOSがハードウェア、割り込み、LCD、Sound、File、USB、Pad、Powerなどを初期化し、それぞれのカーネルサービスベクタを登録する。
2. アプリ起動時に、既定の周期、フォント、Pad repeat、LCD向き、LCDバッファなどを初期化し、アプリのinitializeを呼ぶ。
3. アプリループは `ClockTicks` が次周期に達するまで待つ。DMA使用中は短い待ちを挟み、未使用時はhaltでCPUを休ませる。
4. 周期到達後、USBの遅延処理を進め、Pad状態を更新する。
5. 更新済みPadと電源状態から、アプリ終了、システムメニュー、standby候補を判定する。
6. アプリ終了やシステムメニュー遷移では、アプリ通知、LCDバッファ退避/復帰、heap reset、LCD停止/再転送などをOSが調停する。
7. アプリが実行状態ならperiodic_procを1回呼ぶ。
8. LCD転送は周期ループが自動で行うのではなく、アプリまたはシステム側が明示的に転送APIを呼ぶ。システムメニュー復帰時などはOS側が転送する。
9. 音声処理はアプリ周期とは別に、DMA/割り込み側で再生バッファを進める。アプリはWAVEキューを投入するだけで、実時間出力は音声ドライバが維持する。
10. Timer callbackは1ms割り込みから直接呼ばれるため、アプリperiodic_procより強い制約を持つ。

設計上の要点は、アプリ周期、入力更新、USB進行、LCD flush、音声出力が同じ「毎フレーム関数」に押し込まれていないこと。P/ECEは、入力とUSBを周期ループで進め、音声とタイマを割り込みで維持し、LCDは明示flushにしている。

根拠: `sysdev/pcekn/pcekn.c`, `sysdev/pcekn/runapp.c`, `sysdev/pcekn/timer.c`, `sysdev/pcekn/mainloop.c`, `sysdev/pcekn/pad.c`, `sysdev/pcekn/lcd.c`, `sysdev/pcekn/snd.c`

## 4. アプリに直接触らせない方がよいもの

P/ECEで隠蔽されている、またはAPIは存在しても通常アプリ境界からは遠ざけるべき資源:

| 隠すべきもの | P/ECEでの見せ方 | KotoOSでの判断 | 根拠ファイル |
|---|---|---|---|
| LCD物理転送 | 仮想バッファ + 転送API。Direct転送は通常用途非推奨 | HAL内部。アプリはdraw surfaceとflush requestまで | `sysdev/pcekn/lcd.c`, `docs/API/pceLCDSetBuffer.html`, `docs/API/pceLCDTransDirect.html` |
| PWM/DMA音声 | WAVEキュー、音量、停止API | KotoAudio mixer + backend内部。PWM/PIO/DMAは非公開 | `sysdev/pcekn/snd.c`, `docs/API/pceWaveDataOut.html` |
| USBコントローラ低レイヤ | USB/USBCOM API。D12 endpoint処理は低レイヤ | transport service内部。通常アプリは直接不可 | `sysdev/pcekn/mainloop.c`, `sysdev/pcekn/usbcom.c` |
| Flashセクタ/管理領域 | File API。FAT/Directory/erase/writeは内部 | KotoStorageのsandbox化。物理ブロックは非公開 | `sysdev/pcekn/file.c` |
| 電源制御 | 状態取得、report、standby要求 | Power manager内部。アプリはrequest/subscribe | `sysdev/pcekn/powerman.c`, `docs/API/pcePowerEnterStandby.html` |
| 割り込みベクタ | P/ECEには設定APIがあるが危険 | 通常アプリ不可。デバッグ/カーネル拡張のみ | `sysdev/pcekn/pcekn.c`, `docs/API/pceVectorSetTrap.html`, `docs/API/pceVectorSetKs.html` |
| 予約タイマ | Timer APIはソフトタイマ化して提供 | hardware timerはOS所有。アプリtimerはイベント化 | `sysdev/pcekn/timer.c`, `docs/API/pceTimerSetCallback.html` |
| ADC/電池測定 | Power statusへ集約 | Power HAL内部 | `sysdev/pcekn/powerman.c`, `docs/API/pcePowerGetStatus.html` |
| CPU速度/バスwait | CPU speed APIはあるがLCD保証制約あり | policy manager内部。通常アプリ不可 | `sysdev/pcekn/hard.c`, `docs/API/pceCPUSetSpeed.html` |
| IR送受信タイミング | IRセッションAPI。送受信排他 | KotoIRを作るなら単一ownerセッション | `docs/API/pceIRStop.html`, `docs/API/pceIRStartRxEx.html` |

## 5. KotoOS/PicoCalcへの対応表

| P/ECE予約リソース/抽象 | KotoOS/PicoCalcでの対応 | 所有権の考え方 | 根拠ファイル |
|---|---|---|---|
| LCD DMA/serial transfer | SPI LCD transfer / display flush task | display serviceがSPI/DMA/dirty rectを所有 | `sysdev/pcekn/lcd.c`, `docs/API/pceLCDTrans.html` |
| LCD仮想バッファ | app framebuffer / canvas surface | アプリは描画面を持つが物理flushは要求制 | `docs/API/pceLCDSetBuffer.html`, `sysdev/pcekn/runapp.c` |
| sound PWM/DMA | KotoAudio mixer + PWM/PIO/I2S backend | mixer taskがaudio backendを独占 | `sysdev/pcekn/snd.c`, `docs/API/pceWaveDataOut.html` |
| 1ms tick + app period | system clock + app scheduler tick | kernel/timer service所有。アプリにはtimer event | `sysdev/pcekn/timer.c`, `sysdev/pcekn/runapp.c` |
| Timer callback | app timer event / deferred callback | ISR直呼びではなくrunloop配送 | `docs/API/pceTimerSetCallback.html` |
| pad proc | KotoInput scan/update/repeat | input service所有。アプリはsnapshot/event | `sysdev/pcekn/pad.c`, `docs/API/pcePadGet.html` |
| USB COM | koto monitor/run/install transport | system app/monitor所有。通常アプリとは権限分離 | `sysdev/pcekn/mainloop.c`, `sysdev/pcekn/usbcom.c` |
| file sector API | KotoStorage SD + app save sandbox | 物理SD/FATはstorage service所有 | `sysdev/pcekn/file.c` |
| power/standby | KotoPower suspend/resume manager | 全デバイスを横断するsystem owner | `sysdev/pcekn/powerman.c` |
| battery report overlay | status bar/system overlay | system UIが表示領域を調停 | `sysdev/pcekn/powerman.c`, `sysdev/pcekn/lcd.c` |
| IR exclusive session | future KotoIR session API | 単一owner、排他セッション | `docs/API/pceIRStop.html` |
| trap/KS vector | kernel extension/debug hook | release通常アプリ不可 | `sysdev/pcekn/pcekn.c` |
| CPU speed/bus wait | power policy / performance governor | OS policyのみ | `sysdev/pcekn/hard.c` |

## 6. KotoOSで明文化すべき予約リソース案

| リソース | 所有者 | 優先度 | アプリからのアクセス可否 | hostcall/API経由 | 競合時の方針 |
|---|---|---:|---|---|---|
| CPU0 | KotoOS kernel / app scheduler | 最高 | 通常アプリは実行時間を割当で使用 | app runloop, yield, timer event | watchdogまたはschedulerが回収。長時間占有は警告/停止 |
| CPU1 | system services候補: display/audio/storage/transportのいずれか | 高 | 原則不可 | system service内部のみ | 初期版は所有者を固定。複数serviceで奪い合わない |
| DMA channels | HAL resource allocator | 最高 | 不可 | display/audio/storageなどのhostcall経由 | 静的予約を優先。動的貸出はdeadline/priority付き |
| PIO state machines | HAL resource allocator | 最高 | 通常不可 | audio/display/inputなどのdriver API経由 | 用途別に固定割当。debug buildのみ診断表示 |
| timers/alarms | kernel timer service | 最高 | hardware alarm直接不可。app timer可 | timer hostcall/event | 予約alarmは非公開。アプリtimerは遅延許容キューへ |
| SPI bus | bus manager / display-storage arbitration | 高 | raw SPI不可 | display/storage/device API | LCD flush優先度を明記。SD中はflushを分割/遅延 |
| PSRAM bus | memory manager | 高 | malloc等のみ | allocator/VM API | DMA可能領域と通常領域を区別。直接bus制御不可 |
| SD bus | storage service | 高 | ファイル/sandboxのみ | storage hostcall | 書込中断保護、排他lock、system update優先 |
| UART/USB | monitor/transport manager | 高 | 通常アプリは仮想チャネルのみ。system appは権限付き | console/monitor/install API | monitor優先。通常アプリ転送は切断/一時停止可能 |
| audio output | KotoAudio mixer | 高 | mixer source登録のみ | audio hostcall | backendは単一owner。過負荷時は低優先sourceをdrop/減品質 |
| input scan | KotoInput service | 高 | 状態/イベント購読のみ | input API | scan周期はOS固定。アプリ独自GPIO scanは禁止 |

補足: PicoCalc/RP2040系ではCPU、DMA、PIO、SPIが互いに強く結びつく。P/ECEの教訓は「ハード資源名をアプリAPIに漏らさない」ことよりさらに一歩進んで、「同じ物理資源を使う抽象API同士の優先度をOSが明文化する」ことにある。

## 7. KotoOS設計への結論

### 絶対にHAL内部へ隠すべきもの

- SPI LCDの物理転送、DMA channel、flush完了割り込み
- audio PWM/PIO/I2S backend、audio DMA、出力タイミング
- SD/SPIの物理bus arbitration
- USB/UART transportの低レイヤendpoint/packet処理
- hardware timer/alarmの割当
- PSRAM bus設定、DMA可能メモリ制約
- 電源遷移時のデバイス停止/復帰手順

### hostcallとして公開するもの

- display surface作成、dirty/flush要求、輝度などのポリシー化された表示設定
- audio source登録、buffer enqueue、stop、volume
- input snapshot/event/repeat設定
- app timer、monotonic time、sleep/yield
- app sandbox storage read/write/list
- power status取得、suspend request、active/keepalive通知
- monitor transportの高水準操作。ただし通常アプリ用とsystem用を分ける

### システムアプリだけに許可するもの

- display ownershipの一時奪取、status bar/overlay、system menu
- install/run/debug transport
- system settingsとしてのbrightness、volume、power policy
- storage管理、app install/delete、save migration
- firmware/update関連

### デバッグビルドだけで触れるもの

- DMA/PIO/SPI/timerの予約状況表示
- 割り込みレイテンシ計測
- raw transportログ
- forced suspend/resume test
- panic時のdisplay/audio/input強制停止
- kernel hookやtrap相当の差し替え

### 将来拡張に回すもの

- IR相当の近距離通信API
- 複数アプリ同時実行時のaudio focus/display focus
- per-app USB virtual channel
- storage transaction API
- power budget API
- high precision profiling timer

### 最終判断

P/ECEは、ハードウェア資源が少ないからこそ、アプリに「便利な低レベルAPI」を見せるのではなく、「OSが所有する物理資源」と「アプリが扱う論理オブジェクト」をかなり明確に分けている。LCDはbuffer/flush、音声はWAVE queue、入力は更新済みsnapshot、USBはUSBCOM block、電源はstatus/requestである。

KotoOSではこの方針をさらに強める。特にPicoCalcではSPI、DMA、PIO、timerが複数機能で競合しやすいため、通常アプリにraw busやraw DMAを貸す設計は避ける。アプリAPIは「何をしたいか」を表すhostcallにし、HALは「どの資源で実現するか」を隠す。システムアプリとデバッグビルドだけに低レイヤ観測口を置き、release通常アプリには予約資源の存在を見せない。

推測を含む点: P/ECE IRの内部実装詳細は今回の指定調査対象ではAPI文書中心で確認したため、KotoOS対応は「排他セッション型の通信デバイス」としての設計推測である。またKotoOS/PicoCalcの具体的なDMA/PIO本数配分は、今後の実機HAL実装と性能測定で確定する。
