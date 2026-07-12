# P/ECE SDK Inventory for KotoOS Design Research

## 0. 調査方針

この文書は、`C:\USR\PIECE` 配下のP/ECE SDK/資料一式を、KotoOS設計研究の参考資料として棚卸ししたものです。P/ECE互換ランタイムの作成やSDK資産の移植を目的とせず、設計思想、API分類、開発体験、ツール構成をKotoOS向けに独自表現で要約します。

利用上の前提は `docs/research/piece-analysis/00_usage_policy.md` と同じです。P/ECEのソース、ヘッダ、ライブラリ、サンプル、画像、音声、フォント、文書本文はKotoOSへコピーしません。

## 1. 全体概要

配布物は、P/ECE向けアプリ開発に必要なSDK、サンプル、PC側ツール、Windows用USBドライバ、ハードウェア資料、低レイヤ/システム開発用ソース、更新用イメージ、実行済みアプリ一式を含む統合配布物です。ルート直下には `app`、`bin`、`docs`、`drivers`、`HTML`、`include`、`lib`、`resource`、`sysdev`、`tools`、`update`、`winapp` があり、調査時点で合計約1,900ファイルが確認できます。

開発者向けSDKとしては、`include` の公開ヘッダ、`lib` の静的ライブラリ/起動オブジェクト、`bin` のコンパイラ・リンカ・変換ツール、`docs/API` と `docs/SIMPLE` のAPIリファレンス、`app` のサンプル群が中心です。`tools` にはPC側アプリ/ツールのソースや実行ファイルがあり、`drivers` と `tools/pcedrv` はUSB接続を支えるWindowsドライバ群です。

実機/PC/ツール/ドキュメント/サンプルの関係は、概ね次のように見えます。

- 実機側: `lib/pceapi.lib`、`include/piece.h`、`include/draw.h`、`include/simple.h`、`include/muslib.h`、`include/pclsprite.h`、`app/*.pex`、`update/*.img`
- PC側: `bin/pcc33.exe`、`bin/lk33.exe`、`bin/WinIsd.exe`、`bin/pBMPcnv.exe`、`bin/pPCMcnv.exe`、`bin/FilePack.exe`、`tools/*`
- ドライバ/転送: `drivers/winxp/pcedrvxp.sys`、`drivers/win98/pcedrv2k.sys`、`tools/isd/pieceif.dll`、`tools/pcedrv/*`
- 学習導線: `INDEX.HTM`、`HTML/Tutorial.htm`、`docs/API/index.html`、`docs/SIMPLE/index.html`、`docs/TOOLS/*.htm`
- 低レイヤ研究: `sysdev/pcekn`、`sysdev/pceboot`、`sysdev/music`、`sysdev/ku`

## 2. ディレクトリ別インベントリ

| ディレクトリ | 役割                             | 主な内容                                                                                                                     | 重要度 | 今後見るべき理由                                                                                                                                                                          |
| ------------ | -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `app`        | 実機向けアプリ/サンプル集        | `simple`、`simple_hello`、`sprite`、`adpcm`、`ir`、`menu`、`tank`、`BlackWings`、`odemaru`、`picket`、生成済み `.pex`/`.pfs` | High   | アプリ構造、ビルド単位、起動ファイル、素材配置、ゲーム/ツール系サンプルの開発体験を観察できる。根拠: `app/simple_hello/makefile`、`app/menu/launch.c`、`app/BlackWings/bwings.pex`        |
| `bin`        | SDKコマンド群                    | S1C33系コンパイラ/アセンブラ/リンカ、変換、転送、パック、補助UNIX風ツール                                                    | High   | KotoOS SDKのCLI構成、ビルドパイプライン、アセット変換/転送体験の参考になる。根拠: `bin/pcc33.exe`、`bin/lk33.exe`、`bin/WinIsd.exe`、`bin/FilePack.exe`                                   |
| `DirectX8a`  | Windows依存ランタイム同梱物      | DirectX 8aセットアップ関連                                                                                                   | Low    | P/ECE本体設計ではなく、当時のWindowsツール実行環境向けと思われる。不明点: どのツールが必須依存しているかは未精査。根拠: `DirectX8a/dxsetup.exe`                                           |
| `docs`       | 詳細ドキュメント本体             | API、SIMPLE、muslib、TOOLS、GAMES、APPLICATION、datasheet、回路図、資料                                                      | High   | 仕様抽出の主対象。API分類、ハードウェアモデル、ツール説明、チュートリアルがまとまる。根拠: `docs/API/index.html`、`docs/SIMPLE/index.html`、`docs/回路図/MAIN.CE2`                        |
| `drivers`    | 配布用Windows USBドライバ        | Windows XP/98系の `.sys`/`.inf`、保存版BulkUsb                                                                               | Medium | KotoOS自体ではなくPC連携設計、転送プロトコル、インストール体験の参考。根拠: `drivers/winxp/pcedrvxp.inf`、`drivers/install.txt`                                                           |
| `HTML`       | SDKトップ/入口文書               | トップ、メニュー、チュートリアル、ハード仕様、ドキュメント導線                                                               | Medium | 初学者向け導線とSDK全体の見せ方を把握できる。詳細仕様は `docs` 側のほうが重要。根拠: `HTML/TOP.HTM`、`HTML/Tutorial.htm`、`HTML/Hardspec.htm`                                             |
| `include`    | 公開ヘッダ                       | P/ECE API、描画、SIMPLE、音楽、スプライト、標準C風ヘッダ、CPU関連                                                            | High   | API面の分類と名前空間設計の中心。ただし定義の転記は禁止し、役割分類だけ抽出する。根拠: `include/piece.h`、`include/simple.h`、`include/pclsprite.h`                                       |
| `lib`        | 実機向けライブラリ/起動部        | `pceapi.lib`、`simple.lib`、`sprite.lib`、`muslib.lib`、C標準風ライブラリ、`lib/src`                                         | High   | アプリABI、起動処理、API呼び出し層、ライブラリ分割の研究対象。根拠: `lib/cstart.o`、`lib/pceapi.lib`、`lib/src/pceAppInit` 系ファイル群                                                   |
| `resource`   | SDK付属リソース/生成素材         | フォント生成物、ジョイスティック/音楽画像、フォント作成ツール                                                                | Medium | KotoOSのフォント/内蔵リソース設計の参考。ただし素材・フォント流用は不可。根拠: `resource/font/mkfont.c`、`resource/font/lfont16.bin`                                                      |
| `sysdev`     | ブート/カーネル/低レイヤ開発資料 | `pcekn`、`pceboot`、`music`、`ku` のC/ASM/イメージ/マップ                                                                    | High   | OS/ランタイム設計に最も近い。タイマ、LCD、Pad、USB、ファイル、電源、サウンドなどの構成単位を観察できる。根拠: `sysdev/pcekn/mainloop.c`、`sysdev/pcekn/lcd.c`、`sysdev/pceboot/pceboot.c` |
| `tools`      | PC側ツールの実装/配布            | ISD、画像/PCM変換、FilePack、pcc33、USB通信、ドライバソース                                                                  | High   | KotoOSのホストツール、転送、パッケージング、アセット変換を設計する上で重要。根拠: `tools/isd/isd.c`、`tools/PBMPcnv/src/bmp_conv.c`、`tools/filepack/デコーダsrc/filepack.c`              |
| `update`     | 実機更新イメージ                 | ブート、カーネル、フォント、結合済みイメージ、makefile                                                                       | Medium | システム更新/イメージ構成の参考。内容のバイナリ解析は慎重に扱う。根拠: `update/pceboot.img`、`update/pcekn.img`、`update/all.img`                                                         |
| `winapp`     | Windows連携アプリ/付属アプリ     | `picket`、`おでかけマルチ`、DLL、CHM、データ/動画/音声                                                                       | Low    | SDK/OS中核ではないが、PC連携アプリやユーザー向け配布形態の参考になる。素材流用は禁止。根拠: `winapp/picket/Picket.exe`、`winapp/おでかけマルチ/pieceif.dll`                               |

## 3. 重要ファイル群

### ハードウェア資料

- `HTML/Hardspec.htm`、`HTML/hardspec1.htm`: SDK入口側のハード仕様導線。
- `docs/PIECE ハードウエア割り込み.htm`: 割り込み関連の説明資料。
- `docs/PIECE ポート解説.htm`: ポート/低レイヤI/O関連の説明資料。
- `docs/回路図/MAIN.CE2`、`docs/回路図/IO4.CE2`、`docs/回路図/AUDIO.CE2`、`docs/回路図/PAD.CE2`、`docs/回路図/POWER.CE2`: 回路ブロック別資料。
- `docs/回路図/ALL.CSV`、`docs/回路図/ALL.NET`: 接続/ネットリスト系資料と思われる。
- `docs/datasheet/EPSON/33000Core-J.pdf`、`docs/datasheet/EPSON/s1c33209_221_222j.pdf`: CPU/SoC系データシート。
- `docs/datasheet/EliteMT/LP62S16128-T.pdf`: SRAM系と思われる資料。
- `docs/datasheet/SST/360-39lf-vfx00a-3-ds.pdf`: フラッシュ系と思われる資料。
- `docs/datasheet/Motorola/MC34119.PDF`: オーディオアンプ系と思われる資料。

### APIドキュメント

- `docs/API/index.html`、`docs/API/menu.html`: APIリファレンスの入口。
- アプリ生命周期: `docs/API/pceAppInit.html`、`docs/API/pceAppProc.html`、`docs/API/pceAppExit.html`、`docs/API/pceAppNotify.html`
- LCD/描画: `docs/API/pceLCDSetBuffer.html`、`docs/API/pceLCDTrans.html`、`docs/API/pceLCDPoint.html`、`docs/API/pceLCDLine.html`
- 入力: `docs/API/pcePadGet.html`、`docs/API/pcePadGetDirect.html`、`docs/API/pcePadSetTrigMode.html`
- ファイル: `docs/API/pceFileOpen.html`、`docs/API/pceFileCreate.html`、`docs/API/pceFileReadSct.html`、`docs/API/pceFileWriteSct.html`
- タイマ/時刻: `docs/API/pceTimerSetCallback.html`、`docs/API/pceTimerGetCount.html`、`docs/API/pceTimeGet.html`
- USB/通信: `docs/API/pceUSBCOMSetup.html`、`docs/API/pceUSBCOMStartRx.html`、`docs/API/pceUSBReconnect.html`
- サウンド: `docs/API/pceWaveDataOut.html`、`docs/API/pceWaveStop.html`、`docs/muslib/index.html`
- スプライト: `docs/API/sprite.html`、`docs/API/pclSpriteInit.html`、`docs/API/pclSpriteBGScroll.html`

### アプリ開発チュートリアル

- `HTML/Tutorial.htm`、`HTML/Tutorial1.htm`: SDK付属の入門導線。
- `docs/TUTORIAL/AYAKA/sample.mml`: 音楽/サウンド素材を伴うチュートリアル資料。
- `docs/TUTORIAL/AYAKA/wavetbl/wavetbl.c`: 波形テーブル関連の教材。実装は参照のみに留める。
- `docs/TUTORIAL/TECH/※注意.txt`: チュートリアル補足と思われる注意文書。
- `docs/SIMPLE/index.html`: BASIC風/簡易APIの入口。

### サンプルアプリ

- 最小構成: `app/hello`、`app/simple_hello`、`app/simple`
- 画像/スプライト: `app/sprite`、`app/sprite2`、`app/simple_mc`
- 入力/外部通信: `app/gamepad`、`app/ir`、`app/remote`
- サウンド: `app/adpcm`、`app/tank/snd`、`app/tank/se`
- ランチャー/メニュー: `app/menu`、`app/menu2`
- 実用/ゲーム: `app/picket`、`app/tank`、`app/BlackWings`、`app/odemaru`
- 生成済み実行物集: `app/pex`

### ヘッダ

- コアAPI: `include/piece.h`
- 描画: `include/draw.h`、`include/PIECE_Bmp.h`
- 簡易API: `include/simple.h`
- スプライト: `include/pclsprite.h`
- 音楽/音声: `include/muslib.h`、`include/musdef.h`
- CPU/低レイヤ: `include/s1c33cpu.h`、`include/smcvals.h`
- 標準C風: `include/stdio.h`、`include/stdlib.h`、`include/string.h`、`include/time.h`、`include/math.h`

### ライブラリ

- コアAPI: `lib/pceapi.lib`
- 簡易API: `lib/simple.lib`
- スプライト: `lib/sprite.lib`
- 音楽: `lib/muslib.lib`
- 起動/基本ランタイム: `lib/cstart.o`、`lib/lib.lib`
- C標準風/算術: `lib/string.lib`、`lib/math.lib`、`lib/ctype.lib`、`lib/fp.lib`、`lib/fpp.lib`、`lib/idiv.lib`
- ライブラリ内部資料: `lib/src`

### 低レイヤ/システム開発関連

- カーネル/ランタイム相当: `sysdev/pcekn`
- ブート: `sysdev/pceboot`
- 更新/展開系と思われる領域: `sysdev/ku`
- 音源/波形系: `sysdev/music`
- 代表ファイル: `sysdev/pcekn/mainloop.c`、`sysdev/pcekn/lcd.c`、`sysdev/pcekn/pad.c`、`sysdev/pcekn/timer.c`、`sysdev/pcekn/file.c`、`sysdev/pcekn/usbcom.c`、`sysdev/pceboot/pceboot.c`

### PC側ツール

- SDKコマンド配布: `bin/pcc33.exe`、`bin/as33.exe`、`bin/lk33.exe`、`bin/WinIsd.exe`、`bin/pBMPcnv.exe`、`bin/pPCMcnv.exe`、`bin/FilePack.exe`
- PCツール実装: `tools/isd`、`tools/PBMPcnv`、`tools/PPCMcnv`、`tools/filepack`、`tools/pcc33`、`tools/winisd`、`tools/WinMucc`
- ツール文書: `docs/TOOLS/FilePack.htm`、`docs/TOOLS/pBMPcnv.htm`、`docs/TOOLS/pPCMcnv.htm`、`docs/TOOLS/WinIsd.htm`

### デバイスドライバ

- 配布ドライバ: `drivers/winxp/pcedrvxp.sys`、`drivers/winxp/pcedrvxp.inf`、`drivers/win98/pcedrv2k.sys`、`drivers/win98/pcedrv2k.inf`
- 旧/保存版: `drivers/save/BulkUsb.sys`、`drivers/save/BulkUsb.inf`
- ドライバソース: `tools/pcedrv/xp/sys`、`tools/pcedrv/w2k/sys`
- USB通信補助: `tools/isd/pieceif.dll`、`tools/isd/pieceif.c`、`tools/usbcw`

### 素材/リソース

- フォント: `resource/font`、`update/zfont10.img`、`update/mfont4.img`、`update/lfont16.img`
- SDK画像: `HTML/IMAGE`
- サンプル素材: `app/BlackWings/grp`、`app/BlackWings/snd`、`app/tank/se`、`app/odemaru/datafile`
- ゲーム文書素材: `docs/GAMES/odemaru/img`、`docs/GAMES/tank_image`
- PCアプリ素材: `winapp/おでかけマルチ/ueb`、`winapp/おでかけマルチ/grp`

## 4. KotoOS設計に強く関係しそうな領域

### アプリモデル

P/ECEはアプリ初期化、周期処理、終了、通知、アクティブ応答、別アプリ起動といった役割をAPI文書上で分けているように見えます。KotoOSでは互換名を採用せず、アプリ生命周期、イベント通知、メインループ、終了要求、アプリ間遷移を独自APIとして設計する参考にできます。

根拠: `docs/API/pceAppInit.html`、`docs/API/pceAppProc.html`、`docs/API/pceAppExit.html`、`docs/API/pceAppNotify.html`、`docs/API/pceAppExecFile.html`、`app/menu/launch.c`

### グラフィック

LCDバッファ設定、転送、点/線/矩形相当、向き、輝度、オブジェクト描画、スプライト/BGスクロールがAPI分類として存在します。KotoOSではPicoCalcの表示能力に合わせ、フレームバッファ、部分更新、描画プリミティブ、タイル/スプライト風補助層を分ける設計が有力です。

根拠: `docs/API/pceLCDSetBuffer.html`、`docs/API/pceLCDTrans.html`、`docs/API/pceLCDSetOrientation.html`、`docs/API/sprite.html`、`include/draw.h`、`include/pclsprite.h`

### サウンド

低レベルのWave出力と、MML/音楽ライブラリ系の二層が見えます。KotoOSでは短い効果音、ストリーム/バッファ再生、簡易シーケンサを分け、PC側変換ツールとの関係を設計する価値があります。

根拠: `docs/API/pceWaveDataOut.html`、`docs/API/pceWaveCheckBuffs.html`、`docs/muslib/index.html`、`include/muslib.h`、`sysdev/music`

### 入力

Pad取得、直接取得、トリガモードのような分類があります。KotoOSではキーボード、ボタン、GPIO入力を「現在状態」「エッジ」「リピート/トリガ設定」に分けると、PicoCalc向けにも扱いやすい可能性があります。

根拠: `docs/API/pcePadGet.html`、`docs/API/pcePadGetDirect.html`、`docs/API/pcePadSetTrigMode.html`、`sysdev/pcekn/pad.c`、`app/gamepad`

### ファイル/転送

ファイル作成/削除/検索/セクタ読み書きと、PC側転送/パッケージングツールがまとまっています。KotoOSではアプリパッケージ、保存領域、ホストPC転送、開発時実行のUXを一体で設計する必要があります。

根拠: `docs/API/pceFileOpen.html`、`docs/API/pceFileFindOpen.html`、`docs/API/pceFileReadSct.html`、`docs/API/pceFileWriteSct.html`、`tools/isd`、`tools/filepack`

### 割り込み/タイマ

タイマコールバック、カウント取得、高精度カウント、時刻/アラーム、コンテキストスイッチャ、割り込み資料が見えます。KotoOSでは周期タスク、ゲームループ、スリープ、入力スキャン、オーディオタイミングの基盤として優先度が高い領域です。

根拠: `docs/API/pceTimerSetCallback.html`、`docs/API/pceTimerGetPrecisionCount.html`、`docs/API/pceTimerSetContextSwitcher.html`、`docs/PIECE ハードウエア割り込み.htm`、`sysdev/pcekn/timer.c`

### ランチャー/パッケージ

`.pex`、`.pfs`、`.pid`、`.sav` らしきファイルがアプリ単位で並び、メニュー/ランチャーサンプルもあります。KotoOSでは、実行ファイル、リソース、保存データ、アイコン/メタデータ、ランチャー表示を独自形式で整理する参考になります。

根拠: `app/pex`、`app/menu`、`app/menu2`、`app/picket/picket.pex`、`app/picket/picket.pfs`、`app/BlackWings/bwings.sav`

### PC連携ツール

P/ECE SDKはPC側でビルド、変換、パック、転送、キャプチャ、ドライバを揃える構成です。KotoOSでも単体OSだけでなく、ホストCLI、アセット変換、パッケージング、デバイス転送、ログ/キャプチャをSDK体験として考えるべきです。

根拠: `bin/build.bat`、`bin/run.bat`、`docs/TOOLS/WinIsd.htm`、`tools/isd`、`tools/capture`、`tools/PBMPcnv`、`tools/PPCMcnv`

## 5. 以後の分析優先順位

### Phase 2: API地図の作成

最優先で `docs/API`、`docs/SIMPLE`、`docs/muslib`、`include` を読み、APIをKotoOS独自カテゴリへ分類します。成果物は「機能カテゴリ」「設計意図」「KotoOSで採用/不採用/再設計する点」の表にするのがよいです。

根拠: `docs/API/index.html`、`docs/SIMPLE/index.html`、`docs/muslib/index.html`、`include/piece.h`、`include/simple.h`

### Phase 3: アプリ生命周期とサンプル構成

`app/simple_hello`、`app/simple`、`app/sprite`、`app/adpcm`、`app/ir`、`app/menu` を読み、最小アプリ、描画、入力、音、通信、ランチャーの作法を抽出します。大規模ゲームはこの後でよいです。

根拠: `app/simple_hello`、`app/simple`、`app/sprite`、`app/adpcm`、`app/ir`、`app/menu`

### Phase 4: 低レイヤ/OSモデル

`sysdev/pceboot` と `sysdev/pcekn` を対象に、ブート、メインループ、タイマ、LCD、Pad、ファイル、USB、電源のモジュール分割だけを抽出します。コードの移植ではなく、責務分割と起動順序の研究に留めます。

根拠: `sysdev/pceboot/pceboot.c`、`sysdev/pcekn/mainloop.c`、`sysdev/pcekn/timer.c`、`sysdev/pcekn/lcd.c`、`sysdev/pcekn/file.c`

### Phase 5: PC SDK/転送体験

`bin`、`tools/isd`、`tools/filepack`、`tools/PBMPcnv`、`tools/PPCMcnv`、`docs/TOOLS` を読み、KotoOSのホストツール要件を整理します。特に「ビルド」「アセット変換」「パッケージ」「転送」「デバッグ/キャプチャ」の導線を分けるとよいです。

根拠: `bin/build.bat`、`bin/run.bat`、`tools/isd/isd.c`、`tools/filepack/デコーダsrc/filepack.c`、`docs/TOOLS/FilePack.htm`

### Phase 6: ハードウェア資料の抽象化

`docs/回路図`、`docs/datasheet`、`docs/PIECE ハードウエア割り込み.htm`、`docs/PIECE ポート解説.htm` を読み、P/ECE固有の実装詳細ではなく「小型機器OSに必要な抽象ハードウェア層」を整理します。PicoCalc向けにはCPU/表示/入力/電源/ストレージ/通信の差分表に落とすのがよいです。

根拠: `docs/回路図/MAIN.CE2`、`docs/回路図/AUDIO.CE2`、`docs/datasheet/EPSON/s1c33209_221_222j.pdf`

### Phase 7: 大規模サンプルからUXパターンを抽出

最後に `app/tank`、`app/BlackWings`、`app/odemaru`、`docs/GAMES` を読み、ゲームループ、素材管理、セーブデータ、メニュー、効果音、ドキュメント同梱の実践パターンを抽出します。素材や実装は使わず、KotoOS用サンプルを新規設計するための観察に限定します。

根拠: `app/tank`、`app/BlackWings`、`app/odemaru`、`docs/GAMES/BlackWings.htm`、`docs/GAMES/Tank.htm`

## 6. 不明点/追加確認が必要な点

- `.pex`、`.pfs`、`.pid`、`.sav` の正確な形式と役割は、現時点ではファイル配置からの推定を含みます。
- `DirectX8a` がどのPCツールに必要かは未確認です。
- `drivers/win98` 配下に `pcedrv2k` 名のファイルがあるため、対応OS名と実ファイル名の関係は追加確認が必要です。
- `sysdev` の各モジュールはOS設計上重要ですが、クリーン再設計のため、詳細実装の読み込みは研究メモ化と実装作業を分離して進めるべきです。
