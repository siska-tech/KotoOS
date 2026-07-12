# P/ECE Hardware Model for KotoOS

## 0. Scope

この文書は `C:\USR\PIECE` 配下のP/ECE資料を、PicoCalc/KotoOSのHAL/API/ランタイム設計へ活かすために独自表現で要約したものです。P/ECE互換、資料・回路図・ソースの複製、実装移植は目的にしません。

主な根拠ファイル:

- `HTML/hardspec1.htm`
- `docs/PIECE ハードウエア割り込み.htm`
- `docs/PIECE ポート解説.htm`
- `docs/回路図/ALL.CSV`
- `docs/datasheet/datasheet.txt`
- `docs/API/pceLCDSetBuffer.html`
- `docs/API/pceLCDTrans.html`
- `docs/API/pceLCDTransDirect.html`
- `docs/API/pceWaveDataOut.html`
- `docs/API/pcePadGet.html`
- `docs/API/pcePadGetDirect.html`
- `docs/API/pcePadGetProc.html`
- `docs/API/pceUSBCOMSetup.html`
- `docs/資料/usbcom.txt`
- `docs/API/pceFileReadSct.html`
- `docs/API/pcePowerGetStatus.html`
- `docs/API/pceIRStartTx.html`
- `sysdev/pcekn/hard.c`
- `sysdev/pcekn/lcd.c`
- `sysdev/pcekn/snd.c`
- `sysdev/pcekn/timer.c`
- `sysdev/pcekn/pad.c`
- `sysdev/pcekn/usbcom.c`
- `sysdev/pcekn/d12ci.c`
- `sysdev/pcekn/d12cifs.c`
- `sysdev/pcekn/file.c`

## 1. P/ECEハードウェア全体像

### CPU

P/ECEはEPSON S1C33209を中心にした構成です。資料上の動作クロックは24MHzです。CPU内蔵機能として、8KBの高速内部RAM、DMA、8bit/16bitタイマ、プログラマブルタイマ、シリアルI/F、10bit A/D、汎用I/O、積和演算系の機能が説明されています。

根拠: `HTML/hardspec1.htm`、`docs/回路図/ALL.CSV`、`docs/datasheet/EPSON/s1c33209_221_222j.pdf`

### メモリ

メインRAMは256KBのSRAM、システムおよびファイル保持用に512KBのFlashを持つ構成です。部品表ではSRAMに `A62S7316`、Flashに `SST39VF400A` が見えます。公開資料側ではSRAMデータシートとして `LP62S16128-T.pdf`、FlashデータシートとしてSST系PDFが示されています。

根拠: `HTML/hardspec1.htm`、`docs/回路図/ALL.CSV`、`docs/datasheet/EliteMT/LP62S16128-T.pdf`、`docs/datasheet/SST/360-39lf-vfx00a-3-ds.pdf`

### 表示

LCDは128x88ドット、4階調表示のFSTNです。アプリ側は1ピクセル1バイトの仮想画面バッファを扱い、実際には下位2bitだけを階調として使う設計です。画面への反映は明示的な転送APIで行います。

回路/ポート資料ではLCD向けにシリアルクロック、シリアルデータ、CS、RS、RESETなどが割り当てられており、低レイヤ実装ではDMAとシリアルI/Fを使って転送していることが読み取れます。

根拠: `HTML/hardspec1.htm`、`docs/PIECE ポート解説.htm`、`docs/API/pceLCDSetBuffer.html`、`docs/API/pceLCDTrans.html`、`sysdev/pcekn/lcd.c`

### 音声

音声はCPU側のPWM出力を使うソフトウェア制御型です。出力先は内蔵スピーカーまたはヘッドホン端子です。回路部品表ではオーディオアンプに `MC34119`、スピーカー、ヘッドホン端子が確認できます。

低レイヤ実装ではPWMキャリア生成にタイマを使い、DMAで出力値を流し、割り込み側で次の出力バッファを生成する構成です。Wave APIは8bit PCM、16bit PCM、4bit ADPCM、連続出力、終了コールバック、チャンネル別/マスター音量の概念を持ちます。

根拠: `HTML/hardspec1.htm`、`docs/回路図/ALL.CSV`、`docs/datasheet/Motorola/MC34119.PDF`、`docs/API/pceWaveDataOut.html`、`sysdev/pcekn/snd.c`

### 入力

入力は十字方向、A/B、START/SELECT相当のボタン群です。回路部品表では `PD-LF`、`PD-RI`、`PD-UP`、`PD-DN`、`PD-A`、`PD-B`、`PD-C`、`PD-D` が確認できます。ハード仕様資料では4方向パッド、START/SELECT、A/Bの4ボタンと説明されていますが、API上は8bitのパッド状態として扱われ、C/DがSTART/SELECTへ対応付けられています。

APIは、ハードウェア直接読み取り、周期処理で更新される状態、押下エッジ/トリガを分けています。

根拠: `HTML/hardspec1.htm`、`docs/回路図/ALL.CSV`、`docs/PIECE ポート解説.htm`、`docs/API/pcePadGet.html`、`docs/API/pcePadGetDirect.html`、`docs/API/pcePadGetProc.html`、`include/piece.h`、`sysdev/pcekn/pad.c`

### 通信/転送

PC接続はUSBです。ハード仕様資料は12Mb/sのUSBインターフェースを示し、部品表ではUSB-Bコネクタと `PDIUSBD12` USBコントローラが確認できます。低レイヤ実装ではD12コントローラをメモリマップされたコマンド/データポートとして扱っています。

P/ECEにはPC連携用のUSBCOM APIとPC側DLL/ツールがあり、アプリケーション間通信、実行ファイル転送、ファイル書き込みに使われます。赤外線I/Fもあり、P/ECE同士の通信向けとして説明されています。

根拠: `HTML/hardspec1.htm`、`docs/回路図/ALL.CSV`、`docs/資料/usbcom.txt`、`docs/API/pceUSBCOMSetup.html`、`docs/API/pceIRStartTx.html`、`sysdev/pcekn/d12ci.c`、`sysdev/pcekn/d12cifs.c`、`sysdev/pcekn/usbcom.c`

### ストレージまたはデータ保持

P/ECEは外部SDカードではなく、内蔵Flashをシステム/ファイル領域として使う設計です。ファイルAPIはオープン、作成、検索、削除、セクタ単位読み書きを持ちます。セクタ読み書きAPIでは1セクタ4096バイトと説明されています。

根拠: `HTML/hardspec1.htm`、`docs/API/pceFileReadSct.html`、`sysdev/pcekn/file.c`

### 電源

電源系はUSB給電/電池給電の切り替え、バッテリ電圧計測、アンプ電源、赤外線電源、電源検出をCPUポートとA/Dで扱う構成です。部品表ではレギュレータ、DC-DCコンバータ、電圧検出ICが確認できます。

Power APIは給電元の状態と電池電圧mVを返すモデルです。

根拠: `docs/PIECE ポート解説.htm`、`docs/API/pcePowerGetStatus.html`、`docs/回路図/ALL.CSV`、`docs/datasheet/Ricoh/R1111N-J.PDF`、`docs/datasheet/Ricoh/r1210nxx2-j.pdf`、`docs/datasheet/Ricoh/RN5VD-J.PDF`、`sysdev/pcekn/hard.c`

### 割り込み/タイマ

S1C33209の16bitタイマ、8bitタイマ、プログラマブルタイマ、DMA割り込み、ポート入力割り込みがOS機能に割り当てられています。資料上、1msタイマ、サウンドPWM、USB 6MHzクロック、赤外線送信、LCD転送、サウンド転送などに予約されています。

根拠: `HTML/hardspec1.htm`、`docs/PIECE ハードウエア割り込み.htm`、`sysdev/pcekn/timer.c`、`sysdev/pcekn/lcd.c`、`sysdev/pcekn/snd.c`

### その他I/O

赤外線送受信、外部端子、モニタLED、リセットスイッチ、A/D入力が見えます。ただしユーザーアプリから自由に使えるI/Oは限定され、資料では多くのポートがシステム用途に予約されています。

根拠: `HTML/hardspec1.htm`、`docs/PIECE ポート解説.htm`、`docs/API/pceIRStartTx.html`、`docs/回路図/ALL.CSV`

## 2. 表示系の設計

### LCD接続方式

LCDはCPUのポート/シリアルI/F経由で制御されます。ポート解説ではLCD向けにSCLK、SID、CSB、RS、RESETが割り当てられています。低レイヤ実装ではLCD転送にDMA Ch.0、シリアルI/F Ch.3相当、タイマ由来の転送タイミングを組み合わせています。

根拠: `docs/PIECE ポート解説.htm`、`docs/PIECE ハードウエア割り込み.htm`、`sysdev/pcekn/lcd.c`

### CPUの役割

CPUはアプリが扱う仮想画面バッファをLCDコントローラ向け形式へ変換し、転送開始、転送完了割り込み、向き、輝度、表示開始/停止を管理します。専用GPUや描画アクセラレータは資料からは確認できません。

根拠: `docs/API/pceLCDSetBuffer.html`、`docs/API/pceLCDTrans.html`、`sysdev/pcekn/lcd.c`

### フレームバッファの有無

アプリ/APIレベルには仮想画面バッファがあります。形式は128x88、1ピクセル1バイト、下位2bitで4階調です。文書中に11246バイトと見える箇所がありますが、128x88の計算値は11264バイトです。`HTML/hardspec1.htm` では11264バイトと説明されているため、この文書では11264バイトを採用し、11246は誤記の可能性として扱います。

根拠: `HTML/hardspec1.htm`、`docs/API/pceLCDSetBuffer.html`、`sysdev/pcekn/lcd.c`

### 転送単位/更新方式

通常APIでは `pceLCDTrans()` が画面全体を反映します。低レイヤ実装には範囲転送、直接転送、LCD内部形式相当の転送も見えます。直接形式は128x88/4 = 2816バイトと説明されており、仮想画面バッファとは別のLCD寄り形式です。

根拠: `docs/API/pceLCDTrans.html`、`docs/API/pceLCDTransDirect.html`、`sysdev/pcekn/lcd.c`

### KotoGFX設計への示唆

KotoGFXでは、P/ECEの「アプリが描く論理バッファ」と「LCDに流す物理転送形式」を分ける考え方を採用したいです。PicoCalcは320x320 full-color LCDでSPI接続のため、P/ECEより転送量が大きく、全画面転送だけではCPU時間を圧迫しやすいです。

採用したい設計判断:

- アプリ向けには安定した論理描画面を提供する。
- HAL内部でRGB565/RGB888、ラインバッファ、タイル、差分矩形、DMA/SPI転送を吸収する。
- `present()`、`present_rect()`、`flush_async()`、`wait_flush()` のように、同期/非同期転送の境界をAPI化する。
- PicoCalcではPSRAM 8MBがあるため、フルカラーのバックバッファや複数サーフェスを持てる。ただしSPI帯域が律速になるため、汚れ矩形とライン変換を基本にする。

## 3. 音声系の設計

### 音声生成方式

P/ECEはCPU制御のPWM音声です。低レイヤではタイマでPWMを作り、DMAでPWM比較値へサンプルを流し、割り込み処理で次のバッファを合成します。Wave APIは複数チャンネルのPCM/ADPCMデータを受付け、ソフトウェア側で混ぜて最終出力へ渡す構造と見られます。

根拠: `HTML/hardspec1.htm`、`docs/API/pceWaveDataOut.html`、`sysdev/pcekn/snd.c`、`sysdev/pcekn/sndfast.s`

### アンプ/出力段の役割

PWM波形そのものはCPU/タイマ側で作り、外部のMC34119アンプがスピーカー/ヘッドホン出力を駆動します。ポート解説にはアンプ電源制御があり、低消費電力時に音声系を止める設計が読み取れます。

根拠: `docs/回路図/ALL.CSV`、`docs/datasheet/Motorola/MC34119.PDF`、`docs/PIECE ポート解説.htm`、`sysdev/pcekn/snd.c`

### CPU負荷の推定

推測: P/ECEの音声は専用音源ではなく、PWM/DMA/割り込み/ソフトミキサに依存するため、再生チャンネル数、ADPCM復号、音量処理、連続バッファ管理がCPU負荷になります。ただしDMAで最終出力を流すため、サンプルごとのI/O書き込みをCPUが完全に担当する設計よりは負荷が抑えられています。

根拠: `docs/API/pceWaveDataOut.html`、`sysdev/pcekn/snd.c`

### 割り込み/タイマとの関係

割り込み表ではサウンド転送にDMA Ch.1、PWMに16bitタイマ1が使われています。低レイヤ実装でもDMA転送完了時に次の半分の出力バッファを準備するダブルバッファ的な流れが見えます。

根拠: `docs/PIECE ハードウエア割り込み.htm`、`sysdev/pcekn/snd.c`

### KotoAudio設計への示唆

PicoCalc/KotoOSも音声はCPU制御という前提なので、KotoAudioではP/ECEのように「アプリAPI」と「リアルタイム出力」を強く分離する必要があります。

採用したい設計判断:

- アプリは音声クリップ/ストリーム/シーケンスをキューへ積むだけにする。
- ミキサは固定周期の高優先度タスクまたは割り込み近傍で動かす。
- 出力ドライバはPWM/I2S/PIOなどの実装差を隠すHALにする。
- PCM/ADPCMなどの素材形式はPC側変換ツールで前処理し、ランタイム負荷を抑える。
- バッファ枯渇、レイテンシ、終了コールバック、チャンネル音量、マスター音量をAPIの基本機能にする。

## 4. 入力系の設計

### ボタン/キー構成

P/ECEの入力は8bitのゲームパッド風状態としてまとめられています。物理的には方向4、A/B、START/SELECT相当です。回路部品表では8個のボタン部品として見えます。

根拠: `HTML/hardspec1.htm`、`docs/回路図/ALL.CSV`、`include/piece.h`

### 読み取り方式

ポート解説ではボタン入力がKポートへ割り当てられ、押下時/非押下時の論理値が説明されています。APIでは直接読み取りと、周期処理で更新済みの状態取得を分けます。

根拠: `docs/PIECE ポート解説.htm`、`docs/API/pcePadGetDirect.html`、`docs/API/pcePadGet.html`

### ポーリング/割り込みの関係

通常アプリは直接ハードウェアを読むのではなく、システムがアプリ周期処理の間で呼ぶ更新処理の結果を取得します。この更新処理は前回値との差分からトリガ状態を作ります。資料上、キー入力割り込みはスタンバイ解除にも使われています。

根拠: `docs/API/pcePadGetProc.html`、`docs/API/pcePadGet.html`、`docs/PIECE ハードウエア割り込み.htm`、`sysdev/pcekn/pad.c`

### KotoInput設計への示唆

KotoInputではP/ECEの「raw/current/edge」を採用したいです。PicoCalcはキーボードを持つため、単純な8bitパッドより複雑ですが、入力抽象は同じ層分けで扱えます。

採用したい設計判断:

- `raw` はHAL診断/低レイヤ用。
- `state` は現在押下状態。
- `pressed/released/repeat` はランタイムが周期更新で生成する。
- ゲーム向けには仮想ゲームパッドへマッピングする。
- IME向けにはキーコード、修飾、文字入力、長押し/リピートを分離する。

## 5. 通信・転送・PC連携

### 実機とPCの接続方式

P/ECEはUSB-B端子とPDIUSBD12 USBコントローラを使います。PC側ツール/ドライバ/DLLと実機側USBスタックが対応し、ファイル転送、開発時実行、USBCOMに使われます。

根拠: `docs/回路図/ALL.CSV`、`docs/TOOLS/WinIsd.htm`、`docs/資料/usbcom.txt`、`sysdev/pcekn/d12ci.c`、`sysdev/pcekn/usbcom.c`

### 転送ツールとの関係

P/ECE SDKでは `isd`/`WinIsd` が転送と実行の中心です。USBCOM資料では、実機アプリとPCアプリが互いにバッファを用意し、状態確認をしながら送受信するモデルが説明されています。非同期I/Oというより、状態ポーリングと明示バッファ指定を基本にした設計です。

根拠: `docs/TOOLS/WinIsd.htm`、`docs/資料/usbcom.txt`、`tools/isd`、`sysdev/pcekn/usbcom.c`

### KotoOSのSDカード/USB/開発ツールへの示唆

PicoCalc/KotoOSではSDカードが標準搭載なので、P/ECEの内蔵Flash前提とは違います。KotoOSではPC転送をUSBだけに固定せず、SDカードへのコピー、USB Mass Storage、USB CDC/serial、将来のネットワーク/デバッグブリッジを同じツール体験に束ねる方がよいです。

採用したい設計判断:

- `koto run`: 一時ロード/開発実行。
- `koto install`: SDカード上のアプリ領域へインストール。
- `koto sync`: アセット/セーブ/ログの同期。
- `koto monitor`: ログ、クラッシュ、プロファイル、スクリーンショット。
- 通信APIは、アプリが所有するバッファの寿命、タイムアウト、キャンセルを明確にする。

## 6. PicoCalcとの比較

| 項目 | P/ECE | PicoCalc | KotoOSでの設計判断 |
|---|---|---|---|
| CPU | EPSON S1C33209、24MHz | RP2040 dual-core 133MHz | 片コアをリアルタイム寄り、片コアをVM/UI寄りに使える設計を検討する。 |
| メインRAM | 外部SRAM 256KB、内部高速RAM 8KB | RP2040内蔵RAM + PSRAM 8MB | PSRAMを大容量アセット/フレームバッファへ使い、内蔵RAMを低遅延ワークへ残す。 |
| 不揮発領域 | 内蔵Flash 512KBをシステム/ファイル保持に使用 | SDカード標準搭載 | アプリ、アセット、セーブはSD中心。Flashはブート/設定/最小復旧に限定する。 |
| 表示 | 128x88、4階調FSTN | 320x320 full-color LCD、SPI接続 | 論理サーフェスとSPI転送HALを分離し、部分更新を基本にする。 |
| 表示バッファ | 1px=1byteの仮想バッファ、明示転送 | full-colorでは全画面バッファが大きい | RGB565のライン/タイル/汚れ矩形を標準化し、PSRAMフルバッファは選択式にする。 |
| 表示転送 | CPU変換 + DMA/シリアルI/F | CPU/PIO/DMA/SPIの組み合わせが候補 | `present_rect` と非同期flushをHAL契約にする。 |
| 音声 | CPU制御PWM + アンプ | CPU制御音声 | 高優先度ミキサ、リングバッファ、出力ドライバ分離を必須にする。 |
| 音声API | Waveバッファ、PCM/ADPCM、チャンネル音量 | KotoAudio設計中 | clip/stream/sequence/mixerを分け、PC側変換を前提にする。 |
| 入力 | 8bitゲームパッド風 | PicoCalcキーボード + 追加キー | raw key、text input、virtual gamepad、IMEイベントを分ける。 |
| 通信 | USBコントローラ + PC転送ツール + IR | USB/serial/SD運用が候補 | 開発ツールは接続方式を抽象化し、SD経由も第一級にする。 |
| ストレージAPI | 内蔵Flash上の独自ファイル領域、4096BセクタAPI | SDカードファイルシステム | POSIX風の薄いファイルAPIと、アプリサンドボックス保存領域を分ける。 |
| 電源 | USB/電池判定、電圧測定、周辺電源制御 | PicoCalc側電源設計に依存 | 電源HALは給電元、電圧、充電/低電力、周辺ON/OFFを抽象化する。 |
| 割り込み/タイマ | 1ms、音声、LCD、USB、IRにタイマ/DMAを予約 | RP2040タイマ、DMA、PIO、dual-core | ランタイムtick、audio tick、display DMA、input scanを予約リソースとして管理する。 |
| 開発体験 | PCツールで変換、転送、実行 | KotoOS SDKを新設 | `koto build/asset/pack/run/install/monitor` に分ける。 |
| ランタイム思想 | 小容量機でPC側前処理を活用 | PSRAM/SDありだがCPUとSPIは有限 | 重い変換はPC側、実機は即時性と低メモリ断片化を優先する。 |

## 7. KotoOSで採用したい思想

### 採用したいもの

- 論理バッファと物理転送形式を分ける。
- 表示反映を明示APIにする。
- 入力を直接状態、周期更新状態、トリガ/リピートに分ける。
- 音声をアプリ要求キューとリアルタイム出力に分ける。
- PC側ツールで画像/音声/パッケージ変換を担当する。
- ランチャー、アプリ本体、アセット、保存データを分ける。
- 1ms tick、音声、表示転送など、システム予約リソースを明文化する。

### 変更して採用したいもの

- P/ECEの仮想LCDバッファは4階調前提なので、KotoGFXでは色形式を可変にし、RGB565を基本候補にする。
- P/ECEのWave APIは低レイヤ寄りなので、KotoAudioではclip/stream/sequenceを上位APIに置き、HALへ落とす。
- P/ECEのUSBCOMは状態ポーリング中心なので、KotoOSではタイムアウト、キャンセル、非同期通知を最初から入れる。
- P/ECEの内蔵Flashファイル領域は、KotoOSではSDカード上の通常ファイルとアプリごとの保存領域へ置き換える。
- P/ECEのゲームパッド入力は、KotoOSではキーボード/IME/仮想パッドを統合する入力イベントモデルへ拡張する。

### 採用しない方がよいもの

- Windows専用ドライバ/GUI前提の転送体験。
- ランチャー中核ファイルを通常ファイルとして簡単に壊せる運用。
- 素材を大量にCソースへ変換して埋め込む方式を標準にすること。
- 暗黙の標準リンク設定に依存し、アプリ依存関係が見えにくくなる構成。
- ユーザーアプリがシステム予約タイマ/DMA/I/Oへ直接触る前提。

## 8. 未確認点

### 資料だけでは判断できない点

- LCDコントローラの正確な型番と、全コマンド体系。
- 実機でのLCD転送時間、転送中のCPU停止/並行度。
- 音声ミキサの実効チャンネル数、最悪時CPU負荷、音切れ条件。
- Flashファイルシステムの摩耗対策、空き領域管理、クラッシュ耐性。
- USB転送の実効速度、エラー時の復旧手順。
- 電池駆動時の実消費電流と周辺電源制御の効果。

### 実機確認が必要な点

- LCD全画面転送と範囲転送の体感/計測差。
- 音声再生中に描画/USB/ファイル操作を重ねた場合の挙動。
- ボタン同時押し、チャタリング、長押し/リピートの実挙動。
- USB接続/切断、サスペンド、PCツール異常終了時の復帰性。
- 低電圧時のPower API値とシステムの保護動作。

### 追加で読むべきファイル

- `docs/回路図/ismart047.pdf`
- `docs/回路図/AUDIO.CE2`
- `docs/回路図/IO4.CE2`
- `docs/回路図/MEM2.CE2`
- `docs/回路図/PAD.CE2`
- `docs/回路図/POWER.CE2`
- `docs/API/pceLCDTransDirect.html`
- `docs/API/pceLCDTransRange.html`
- `docs/API/pceWaveCheckBuffs.html`
- `docs/API/pceWaveDataCheckBuffs.html`
- `docs/API/pceTimerSetCallback.html`
- `docs/API/pceTimerGetPrecisionCount.html`
- `docs/API/pcePowerEnterStandby.html`
- `sysdev/pcekn/mainloop.c`
- `sysdev/pcekn/powerman.c`
- `sysdev/pcekn/irsub.c`
- `tools/isd`
- `tools/usbcw`
