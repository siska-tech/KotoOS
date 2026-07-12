# **PicoCalcハードウェア仕様およびKotoOSアーキテクチャ設計・実現性評価レポート**

> **追補（2026-06-13）**: 本レポート内には C/C++（Pico SDK）を前提とした初期検討記述が含まれるが、KotoOS の開発方針は Rust を主要実装言語とする。C/C++ は既存ドライバ、FatFs、PSRAM ライブラリ、VM 等を利用する必要がある場合の FFI 対象として扱い、最新の開発方針は [REQUIREMENTS.md](REQUIREMENTS.md)、[ARCHITECTURE.md](../architecture/ARCHITECTURE.md)、[HAL_API.md](../architecture/HAL_API.md) を正とする。

## **エグゼクティブサマリ**

本レポートは、ClockworkPi社製のMCU搭載携帯端末「PicoCalc」をターゲットプラットフォームとし、P/ECE（ピース）風の小型アプリケーション実行環境ならびに軽量日本語PDA環境を統合した「KotoOS」を開発するための、包括的なハードウェア調査およびアーキテクチャ設計を提供するものである。  
分析の結果、PicoCalcはRaspberry Pi Pico系メインMCUと、キーボードや電源管理を担うSTM32サウスブリッジMCUによるデュアルMCU構成を採用していることが確認された。この設計は拡張性に優れる一方で、SPI接続のLCDディスプレイ（320×320解像度）への描画帯域幅の限界、RP2040における内部SRAM（264KB）の枯渇、PIO（Programmable I/O）駆動によるPSRAMアクセスのレイテンシ、そして同一PWMスライスを共有する音声出力ピンといった、特有のハードウェア的制約を抱えている。  
KotoOSが目標とする、フルカラー320×320画面でのビジュアルノベルやDOS風ミニゲームの実行、SKK風の軽量日本語入力、およびPCシミュレータとの共通コード実行を達成するためには、ベアメタルに近いC/C++（Pico SDK）によるHAL（Hardware Abstraction Layer）の厳格な分離設計が不可欠である。特に、画面全体を毎フレーム更新するフルフレームバッファ方式を廃し、差分描画（Dirty Rectangles）やスキャンライン単位でのDMA転送を軸とした描画戦略を採用する必要がある。また、アプリケーションの実行形態としては、ネイティブバイナリの動的ロードが困難なメモリ制約を考慮し、WebAssemblyや軽量バイトコードVM（Virtual Machine）を採用することが推奨される。本レポートでは、これらの制約を克服するための具体的なシステムアーキテクチャと、段階的なMVP（Minimum Viable Product）実装ロードマップを提示する。

## **1\. PicoCalcの基本ハードウェア仕様**

PicoCalcは、汎用的なRaspberry Pi Picoシリーズをコアモジュールとして採用しつつ、ペリフェラル制御専用のサウスブリッジMCUを組み合わせた複合的なハードウェア構造を持つ 1。

| 仕様項目           | 詳細内容                                                                        | 備考・制約                                                                                                   |
| :----------------- | :------------------------------------------------------------------------------ | :----------------------------------------------------------------------------------------------------------- |
| **搭載メインMCU**  | Raspberry Pi Pico 1H / 1WH (RP2040) Pico 2H / 2W (RP2350)                       | コアモジュールの差し替えにより、CPU性能およびRAM容量のハードウェア的なアップグレードが可能である 2。         |
| **CPU / RAM性能**  | RP2040: Cortex-M0+ (133MHz) / 264KB RAM RP2350: Cortex-M33 (150MHz) / 520KB RAM | メモリ領域の枯渇がOS設計上の最大のボトルネックとなる。                                                       |
| **Flash容量**      | 2MB 〜 16MB                                                                     | 搭載するPicoモジュールに依存する（標準キットのPico 1Hは2MB） 3。OSコアと内蔵アセットの配置場所となる。       |
| **基板側PSRAM**    | 8MB (SPI接続、QSPI物理配線)                                                     | RP2040搭載時はハードウェア制約によりPIO経由でアクセスするため、メモリ空間への直接マッピング（XIP）は不可 4。 |
| **画面仕様**       | 4インチ IPS LCD, 320×320ピクセル                                                | SPIインターフェース接続。ILI9488またはST7365P互換コントローラ 5。                                            |
| **キーボード仕様** | 67キー QWERTY配列（バックライト付）                                             | STM32サウスブリッジ経由で制御され、メインMCUとはI2C（アドレス0x1F）で通信する 2。                            |
| **音声出力仕様**   | デュアルPWMスピーカー                                                           | 左右チャンネルが同一のPWMスライスに接続されているため、ハードウェアレベルでの独立した周波数制御が不可能 7。  |
| **SDカード仕様**   | MicroSDカードスロット (SPI接続)                                                 | SPI0バスを使用。FATファイルシステムによるアクセスが基本となる 8。                                            |
| **電源・充電管理** | 18650 リチウムイオン電池, USB Type-C充電                                        | バッテリー充放電は専用回路およびSTM32で管理され、I2C経由で残量取得が可能 2。                                 |

### **MCUの差し替え可否と拡張性**

PicoCalcの設計は「アップグレードフレンドリー」を標榜しており、標準で同梱されるPico 1H（RP2040）から、より高性能なPico 2（RP2350）や、Wi-Fi/Bluetoothを搭載したPico W系モジュールへの差し替えが公式にサポートされている 2。Pimoroni製のPico Plus 2Wなど、独自に大容量PSRAMやFlashを搭載した互換ボードへの換装事例も報告されている 10。KotoOSの設計においては、将来的な性能向上（RP2350への移行）を見据えつつも、初期ターゲットとして最も制約の厳しいRP2040上で安定動作するメモリフットプリントを維持することが求められる。

### **GPIOピンマッピングとハードウェア制約**

PicoCalcメインボード上では、主要なペリフェラルが特定のGPIOピンにハードワイヤードで割り当てられている。開発においては以下のピンアサインを厳格に順守するAPI設計が必要である 8。 ディスプレイ通信用のSPI1バスは、SCK=GP10、MOSI=GP11、MISO=GP12、CS=GP13、DC=GP14、RESET=GP15に割り当てられている。ストレージアクセスの要となるSDカードはSPI0バスを使用し、MISO=GP16、CS=GP17、SCK=GP18、MOSI=GP19、DETECT=GP22の構成である。基板上のPSRAMはPIOでの駆動を前提とし、CS=GP20、SCK=GP21、MOSI=GP2、MISO=GP3を使用する 4。さらに、キーボード制御やバッテリー管理を担うSTM32サウスブリッジとはI2C1（SDA=GP6, SCL=GP7）で接続され、音声出力用のPWM信号は左チャンネルがGP26、右チャンネルがGP27に固定されている 8。これらのピンアサインは既存のGPIO拡張用途を大きく制限するため、外部センサー等を追加する余地は少ない。

## **2\. 画面・LCDディスプレイ周辺の仕様と描画戦略**

### **LCDコントローラの仕様と実効転送速度**

搭載されている4インチIPSディスプレイは、ILI9488またはそれとコマンド互換性を持つST7365Pコントローラによって駆動される 5。インターフェースは4線式SPI（DBI Type-C Option 3）が採用されている 11。データシート上のSPIクロックの公式な上限は20MHzと規定されているが、セットアップ時間およびホールド時間の物理的マージンを利用し、ファームウェアの実装によっては40MHzから最大75MHz程度までオーバークロックして駆動する事例が多数報告されている 12。  
しかし、SPI接続におけるデータ転送帯域は、320×320ピクセルのフルカラー更新において致命的なボトルネックとなる。16-bitカラー（RGB565）を使用した場合、1フレームあたりのデータ量は204,800バイト（320×320×2バイト）に達する。クロックを限界の40MHzまで引き上げ、DMA（Direct Memory Access）を駆使してCPUの介在なく連続転送を行ったとしても、理論上の上限フレームレートは約24.4fpsであり、各種オーバーヘッドを含めると画面全体のベタ塗り更新は15〜16fps程度が現実的な限界値となる 12。

### **フレームバッファと差分描画の設計方針**

RP2040の内部SRAMは264KBに制限されており、204KBのフルスクリーン・フレームバッファを保持することは、OSカーネルやアプリケーションのヒープ領域を枯渇させるため不可能である。したがって、KotoOSでは用途に応じた以下の描画戦略を動的に切り替えるアーキテクチャが必須となる。

1. **差分描画（Dirty Rectangles）戦略**: メモ帳、ファイラ、ランチャー（KotoShell）などのGUIアプリケーションでは、画面全体を更新する必要はない。UI上の変更が生じた矩形領域（Dirty Rectangle）のみを算出し、ILI9488のウィンドウアドレス設定コマンドを用いて部分的なピクセルデータのみを転送する。この手法により、SPI転送量を劇的に削減し、PCライクな滑らかなカーソル移動や文字入力を実現する。  
2. **ラインバッファ・スキャンラインレンダリング**: アクションゲームやビジュアルノベルのトランジションなど、動的な描画が必要な場面では、フルフレームバッファを持たず、数ライン分（例: 320ピクセル×16ライン×2バイト \= 10KB）のバッファを2面用意する（ダブルバッファリング）。CPUが1つのバッファにラスタライズを行っている間に、DMAがもう1つのバッファをノンブロッキングでSPI送信する。

### **仮想画面モードと性能見積もり**

KotoOSがサポートすべき各画面モードのメモリ消費量と性能見積もりを分析する。

| 画面モード             | 用途                        | VRAM見積もり                 | 転送戦略とfps予測                                                                                                                               |
| :--------------------- | :-------------------------- | :--------------------------- | :---------------------------------------------------------------------------------------------------------------------------------------------- |
| **320×320 フルカラー** | KotoShell, ビジュアルノベル | 約200KB (全画面保持は不可)   | 差分描画、またはJPEG/PNGデコード時の直接ブロック転送。UI更新なら30fps以上、全画面更新は15fps程度。                                              |
| **160×160 仮想画面**   | P/ECE風アプリ, レトロゲーム | 51.2KB (内部SRAMに保持可能)  | 内部SRAM上にVRAMを確保し、CPUまたはPIOで縦横2倍にNearest Neighbor拡大しながらDMA転送。転送量は全画面と同じだが、描画負荷は軽く30fps前後が可能。 |
| **320×200 DOS風画面**  | DOS風ミニゲーム, RPG        | 128KB (内部SRAMの半分を消費) | 画面上部320×200のみをゲーム画面として頻繁に更新（SPI転送量約62%減）。下部320×120は静的なUIとして保持。25〜30fpsの達成が現実的。                 |
| **RGB111 8色モード**   | 高速アクション, PicoMings   | 320×320で 51.2KB             | ILI9488の3ビットモードを利用し、2ピクセルを1バイトにパックして送信 12。転送速度が劇的に向上し、フル解像度でも60fpsに迫る描画が可能。            |

既存のPicoCalc向けプロジェクト（uMacエミュレータなど）においても、16-bit RGB565モードを用いたDMA転送と差分更新を組み合わせることで、実用的なレスポンスが達成されている 13。

## **3\. キーボード・入力インターフェースの実態と最適化**

### **I2Cキーボードコントローラの仕様と遅延問題**

PicoCalcのキーボードは67キーのQWERTY物理配列を備えており、入力を監視するマトリクススキャンはメインMCUではなく、STM32サウスブリッジMCUが担当している 2。メインMCU（RP2040）はI2Cバス（アドレス0x1F）を介してSTM32からキーの押下状態をポーリングする仕組みである。 公式の初期ファームウェアや一部のライブラリでは、このI2C通信クロックが10kHzに極端に制限されている事例があり、これがポーリング頻度の低下や入力レイテンシの増大（最大数十ミリ秒の遅延）を引き起こす原因となっていた 15。しかし、カスタムBIOSの導入やPicoMiteの最新ファームウェアでは、I2Cクロックを100kHzまたは400kHzに設定可能であることが実証されている 15。KotoOSの入力API（KotoSDK）では、起動時にI2Cバスクロックを最低でも100kHzに初期化し、毎フレーム（約16.6ms周期）のゲームループ内でノンブロッキングにキー状態を取得するポーリング設計が必須である。

### **キーマトリクスの制約とゲーム入力への応用**

物理的なマトリクス配線の構造上、特定のキーの組み合わせにおいてゴースト現象（意図しないキーの入力判定）やブロッキング（同時押しの無効化）が発生する。PicoCalcをゲーム機として利用する場合、方向キー（十字キー）と複数のアクションキー（A, B, X, Y相当）を同時に押下するシチュエーションが頻発する。KotoOSでは、デフォルトのゲームパッド割り当てとして、マトリクス干渉の少ないキー群（例: 方向キー独立、アクションキーに特定の記号キーや端のキーを割り当てる）をOSレベルでマッピングする機能を提供する。PicoMings（レミングス風ゲーム）のようにマウスカーソル的な操作が主体のゲームにおいては、方向キーとEnter/Spaceキーの組み合わせのみで完結するため、入力制約は問題とならない。

### **SKK風IMEの実装と親指タイピングの人間工学**

KotoOSの最大の特長である「日本語入力」において、SKK（Simple Kana to Kanji conversion）方式の採用は極めて合理的である。一般的な連文節変換IMEは、数十MBに及ぶ辞書データと複雑な形態素解析エンジンを必要とし、264KBのSRAMでは到底動作しない。対してSKKは、「大文字から始まる入力（例: Kanja）」を変換対象とするシンプルなアルゴリズムであり、メモリ消費を最小限に抑えることができる。  
しかし、PicoCalcは両手で本体をホールドし、親指でタイピングするフォームファクタである。PC用キーボードのように「小指でShiftキーを押下したまま、別の指で文字キーを叩く」といった同時押し操作は、親指タイピングにおいて著しく操作性を損なう。この人間工学的な課題を解決するため、KotoIMEには「Sticky Shift（スティッキー・シフト）」機能の実装が不可欠である。これは、Shiftキーを一度押して離すと、OS内部で次の1ストロークのみが大文字入力としてロックされる機能である。これにより、親指のみの逐次入力でスムーズなSKK変換トリガーを発動させることが可能となる。

## **4\. ストレージとファイルシステムのアーキテクチャ**

### **SDカードの接続方式と転送性能**

PicoCalcの外部ストレージはMicroSDカードスロットであり、SPI0バス（GP16〜GP19, GP22）を通じて接続される 8。SDIO（Secure Digital Input Output）モードではなく、レガシーなSPIモードで通信を行うため、物理的な帯域幅による速度制限が存在する。Pico SDKの標準的なSPI制御を用いた場合、読み書きの実効速度は数百KB/s〜数MB/s程度に留まる。また、一部の低品質なSDカードや大容量SDXCカードにおいて、高いSPIクロックレートでの初期化に失敗するハードウェア相性問題がフォーラムで多数報告されている 10。

### **ファイルシステムの設計とFatFsの統合**

ファイルシステムには、組み込み環境でデファクトスタンダードとなっているFatFsライブラリを採用する。FatFsを利用することで、FAT16およびFAT32形式でフォーマットされたSDカードに対し、標準的なC言語のストリームI/O（fopen, fread等）に近い感覚でアクセスが可能となる 16。 KotoFS（KotoOSのファイル管理API）を設計する上での最大の注意点は、SPIモードによるランダムアクセスのペナルティである。SDカード上の小さなファイル群（アイコン、個別の音声ファイル、細かなテキストデータ）を頻繁に開閉・シークする動作は、FATクラスタチェーンのトラバースとSPI通信のオーバーヘッドを増大させ、著しいパフォーマンス低下（カクつき）を引き起こす。

### **アプリパッケージとリソース配置の最適化**

上記の問題を回避するため、KotoOSで動作するアプリケーション（P/ECE風アプリ、ビジュアルノベル等）は、単一のアーカイブ形式（例えば .kpa \- Koto Package Archive 形式）に統合して配布・実行する設計とする。

* **シーケンシャルリードの徹底**: アセットデータ（画像、MML、フォント）はパッケージ内で連続したセクタに配置し、OSは実行開始時に必要なブロックを一括してSRAMまたはPSRAMにシーケンシャルリード（事前ロード）する。  
* **SKK辞書の扱い**: SKKの変換辞書（数MBに及ぶ巨大なテキストファイル）をSRAMに載せることはできないため、SDカード上に配置した辞書ファイルに対してバイナリサーチ（二分探索）を行う。この際、ファイルポインタのシーク操作に伴う遅延を最小化するため、辞書のインデックス（各行の先頭文字のファイルオフセット位置）のみを数百バイトのテーブルとしてSRAMに常駐させる手法が有効である。

## **5\. メモリ構成の深掘りとPSRAM活用戦略**

KotoOSの開発において、メモリ管理はアーキテクチャの根幹を成す最も難易度の高い領域である。PicoCalcはデュアルメモリ構造（内部SRAM \+ 外部PSRAM）を持つが、その接続方式に起因する深刻な制約が存在する。

### **RP2040内部RAMの枯渇問題**

RP2040の内部SRAMは物理的に264KBである。KotoOSをC/C++で実装した場合、OSのカーネル機能、FatFsのバッファ、USBシリアル通信スタック、スタックメモリ、ヒープ領域等で恒常的に約80KB〜100KBが占有される。残された約150KB〜180KBの空間内で、アプリケーションの変数、フレームバッファ、フォントキャッシュをやり繰りする必要がある。このため、フル解像度（320×320）の16-bitフレームバッファ（200KB）をSRAM上に確保することは物理的に不可能である。

### **PIO駆動PSRAMの制約と限界**

PicoCalcメイン基板には8MBのPSRAMが搭載されているが、RP2040の仕様上、標準のSPIペリフェラルでは高クロックでのPSRAMアクセスに必要な厳密なタイミング要件（クロック立ち下がりでのサンプリングやダミーサイクルの挿入）を満たすことができない。そのため、rp2040-psramライブラリ等を用い、PIO（Programmable I/O）のステートマシンを駆使してソフトウェア的にSPIプロトコルをエミュレーションし、DMAと連携させてデータを転送する手法が取られている 4。 このPIO駆動アプローチには致命的な制約がある。それは、**PSRAMをCPUのメモリ空間に直接マッピング（Execute in Place: XIP / Memory-Mapped）できない**という点である 4。すなわち、C言語のポインタ（char c \= \*psram\_ptr;）で直接データを読み書きしたり、PSRAM上に置いたネイティブコードの関数（命令）を直接CPUにフェッチさせて実行したりすることは不可能である。必ずアクセス用のAPI（psram\_read(), psram\_write()）を呼び出し、内部SRAMのバッファへブロック転送する必要がある。

| 開発言語/環境             | PSRAM利用の可否と実装方式                                                                                                                                       |
| :------------------------ | :-------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **C / C++ (Pico SDK)**    | PIOとDMAを活用したライブラリ（rp2040-psram等）を組み込むことで、非同期の外部ストレージとして自由に活用可能 4。                                                  |
| **Rust**                  | embassy-rp等のHALを利用し、I2CやPIOを制御するコードが存在するが、PSRAM用のPIOステートマシンは自作またはC言語ライブラリのバインディングが必要 19。               |
| **MicroPython / MMBasic** | MicroPythonのカスタムビルドではPSRAM対応事例があるが、公式のPicoMite（MMBasic）ではデフォルトで無効化されており、直接的なメモリアクセス拡張としては利用不可 4。 |

### **PSRAMのリソース割り当てと実現性**

ポインタアクセスが不可能という制約を踏まえ、PSRAMは「超高速なスワップ・キャッシュストレージ」として割り切って運用する。

* **仮想フレームバッファ**: OSの裏画面やウィンドウのバックキングストアとしてPSRAMを使用する。描画時、CPUは内部SRAMのラインバッファ（16ライン分等）を処理し、DMAを用いてPSRAMから必要な背景ピクセルを取り寄せ、合成後にLCDへSPI送信する。  
* **日本語フォントキャッシュ (KotoFont)**: JIS第1水準・第2水準を網羅する16×16ピクセルのビットマップフォント（約300KB）や、8×12フォントをPSRAMに常駐させる 20。テキスト描画要求が発生した際、該当する文字コードのグリフデータのみをPSRAMからSRAMへ数十バイト単位で読み出し、ラスタライズを行う。  
* **ビジュアルノベル (KotoVN) とアセット**: 背景画像や立ち絵、音声PCMデータをステージ・シーン単位でPSRAMに事前ロードする。SDカードのランダムアクセス遅延を隠蔽し、シームレスなシーン切り替えを実現する。

*注記*: PicoモジュールをRP2350搭載のPico 2へ差し替えた場合、またはPimoroni Pico Plus 2W等を利用した場合、QSPIインターフェースを通じてPSRAMを直接メモリ空間にマッピング可能となる可能性があり 4、この場合はアーキテクチャの自由度が飛躍的に向上する。

## **6\. 音声出力機構と音楽再生の限界突破**

### **ハードウェア設計の致命的制約**

PicoCalcはデュアルスピーカーを備え、迫力あるサウンド出力が可能であるように見えるが、内部配線において極めて厄介な制約を抱えている。基板の左チャンネルスピーカーはGP26に、右チャンネルスピーカーはGP27に接続されている。RP2040の仕様上、GP26とGP27は全く同じPWMスライス（Slice 5）を共有している 7。 PWMペリフェラルのアーキテクチャ上、同一スライスに属する2つのピン（チャンネルAとチャンネルB）は、独立した「デューティ比（音量）」を設定することはできるが、「ラップ値（周波数・音程）」はスライス全体で1つしか設定できない。したがって、左右のスピーカーから異なる音階のBEEP音（矩形波）を同時に鳴らして和音を構成するハードウェア的なステレオ出力は不可能である 7。

### **ソフトウェアPCMミキサーによるMMLと効果音の実現**

このハードウェア制約を突破し、KotoMML（複数トラックのMML再生）やポリフォニックなゲーム効果音（PCM再生）を実現するためには、ハードウェアPWMによる直接発音を放棄し、**ソフトウェアPCMミキサー方式**を採用する必要がある。

1. **PWMのキャリア周波数固定**: PWMスライスのラップ値を極端に小さく設定し、キャリア周波数を可聴域をはるかに超える超音波帯域（例: 64kHz〜128kHz）に固定する。  
2. **DMAとタイマー駆動による波形合成**: CPUのコア1（セカンダリコア）をオーディオ処理専用に割り当てるか、高精度タイマー割り込み（例: 22.05kHzや32kHz）を使用する。割り込みごとに、メモリ上の複数の波形データ（MMLエンジンの矩形波・三角波ジェネレータ、またはPCMサンプル）をソフトウェア上で加算・合成（ミキシング）し、その瞬間的な振幅値をPWMの「デューティ比」レジスタに書き込む。  
3. **ストリーミングバッファの運用**: オーディオ出力が途切れないよう、リングバッファ構造を採用し、DMAを用いてSRAMからPWMレジスタへ連続的にサンプル値を送り込む。

このアプローチにより、貴重なCPUサイクルはある程度消費されるものの、レミングス風ゲーム（PicoMings）でのBGMと効果音の同時発音や、レトロPCライクな高機能MMLプレイヤーの構築が完全に可能となる。既存のPico用オーディオライブラリ（PicoAudio等）の知見をそのままKotoSDKに統合できる。

## **7\. 開発環境と既存プロジェクトからの技術的示唆**

KotoOSの開発において、車輪の再発明を避けるためには、活発なPicoエコシステムと既存の移植事例を最大限に活用すべきである。

### **既存オペレーティングシステムとエミュレータの移植例**

PicoCalc上では既に多数の高度なソフトウェアが稼働しており、ハードウェアの限界を引き出す実装手法の宝庫となっている。

* **Fuzix OS**: 8ビット/16ビットプロセッサ向けの軽量UNIX系OSであるFuzixがPicoCalcに移植されている 22。Fuzixは限られたRAM空間で複数のプロセスをスワップしながら実行し、SDカード上のファイルシステムをマウントする。これはKotoOSにおけるプロセス管理とKotoFSの堅牢性を裏付ける証明である。  
* **uMac (Macintosh Emulator)**: 初期MacintoshのOS（System 7）をエミュレートするプロジェクトが存在する 14。このエミュレータは、数十KBの内部SRAMの限界を越えるため、SDカードとPSRAMを仮想メモリのように扱い、緻密なメモリアクセスのオーケストレーションを行っている。KotoRuntimeの仮想マシン設計において、ページング機構の強力な参考資料となる。  
* **MachiKania / BASIC系環境**: PicoMite（MMBasic）等の環境では、REPL（対話型評価環境）とSDカードからのスクリプト実行が確立している 21。KotoShellのコマンドパーサや、KotoRuntimeをスクリプトインタプリタとして実装する場合のアーキテクチャモデルとなる。

### **開発・デバッグ手法**

KotoOS本体のOSカーネルやHAL層は、Pico SDKを用いたC/C++でのクロスコンパイルによって開発する。ビルドされたバイナリはUF2（USB Flashing Format）ファイルとして出力され、PicoCalcをBOOTSELモード（本体基板上のボタンまたは特定のキーコンビネーションで起動）でPCにUSB接続し、ドラッグ＆ドロップするだけでフラッシュメモリに書き込まれる。デバッグ情報は、標準出力をUSB-CDC（仮想シリアルポート）にリダイレクトし、PC上のターミナルソフト（CoolTermやTeraTermなど）でリアルタイムにログを監視する手法が一般的かつ最も効率的である 25。

## **8\. KotoOS向け機能の実現性評価**

ここまでのハードウェア・ソフトウェアの調査結果に基づき、ユーザーが要件として掲げたKotoOSの各主要機能について、PicoCalc実機上での実現性、開発難易度、および推奨される実装方針をマトリクス評価する。

| 機能モジュール  | 目的 / 概要                | 実現性     | 難易度 | 制約事項と推奨実装方針                                                                                                                                                                                                          |
| :-------------- | :------------------------- | :--------- | :----- | :------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **KotoShell**   | P/ECE風ランチャー          | 極めて高い | 低     | C/C++のネイティブコードでOS本体に組み込む。KotoFSを用いてSDカード内の.kpa（パッケージ）一覧を取得し、十字キーとEnterで選択する軽量なUIを構築する。                                                                              |
| **KotoRuntime** | 小型アプリ実行環境         | 高い       | 極高   | **最大の技術的課題**。PSRAM上のネイティブバイナリを直接実行できないため、OS内に**軽量バイトコードVM**（Wasm3、mruby、または独自設計のスタックマシン）を組み込み、インタプリタ形式でアプリを実行するアーキテクチャが必須である。 |
| **KotoSDK**     | 画面/入力/音/ファイルAPI   | 極めて高い | 中     | C言語によるシステムコール/HAL層。画面描画はコールバックによるライン更新、入力は構造体ポーリング、音声はオーディオバッファ要求のインターフェースを提供する。                                                                     |
| **KotoIME**     | ローマ字かな＋SKK風入力    | 高い       | 高     | P/ECEやTiPOの系譜を受け継ぎ、画面下部に1〜2行の専用IME入力ラインを常時確保する設計が望ましい。Sticky Shiftの確実な捕捉と、PSRAMまたはSD上の辞書検索の高速化が鍵。                                                               |
| **KotoFont**    | 日本語フォント描画         | 極めて高い | 中     | 「美咲フォント」や「k8x12」などの軽量ビットマップフォント 21 をSRAM/PSRAMにロードし、文字コードからオフセットを計算してピクセルをプロットする。                                                                                 |
| **KotoFS**      | アプリ/文書/セーブ管理     | 極めて高い | 低     | FatFsのラッパーとして実装。セキュリティとデータ破損防止のため、アプリからは自身のSandboxディレクトリ（特定フォルダ配下）のみアクセス可能な仮想パス機構を提供する。                                                              |
| **KotoVN**      | ビジュアルノベルエンジン   | 高い       | 中     | KotoRuntime上で動作する専用スクリプトパーサ。背景画像は256色（インデックスカラー）で独自のRLE（連長圧縮）等を施し、SDからPSRAMへ展開後、必要な矩形のみをSPI描画する。                                                           |
| **KotoMML**     | MML/BEEP音楽環境           | 高い       | 高     | 前述の通り、ハードウェアPWMの制約を回避するため、ソフトウェアPCMミキサーエンジンをバックグラウンド（タイマー割り込み）で駆動する。                                                                                              |
| **KotoDOS**     | 320×200 DOS風ゲーム        | 高い       | 中     | 画面上部の320×200ピクセル領域のみを更新ターゲットとする。内部SRAMに128KBのラインバッファ/VRAMを確保できれば、最高速での転送が可能。                                                                                             |
| **PicoMings**   | レミングス風ゲーム         | 高い       | 高     | 多数のキャラクター（スプライト）が独立して動くため、背景タイルマップとの合成をスキャンライン単位で実行する軽量な2Dスプライトエンジンが必要となる。                                                                              |
| **KotoSim**     | PC上のPicoCalcシミュレータ | 極めて高い | 低     | SDL2（Simple DirectMedia Layer）ライブラリを用いて、PC上に320×320の仮想ウィンドウを生成。実機到着前に開発の80%をPC上で完結させる。                                                                                              |

## **9\. 推奨アーキテクチャおよびMVP実装ロードマップ**

KotoOSを安定的かつ継続的に発展させるための、全体アーキテクチャ設計と段階的な開発ロードマップを提案する。

### **9.1 HAL（Hardware Abstraction Layer）の厳格な分離**

PC上のシミュレータ（KotoSim）とPicoCalc実機で単一のソースコードをコンパイル可能にするため、ハードウェア依存コードを完全に隔離するHAL層を設計する。コアOSロジック（UI、ファイル管理、VM、文字描画）はピュアなC99/C++11で記述する。

* **PCシミュレータ用HAL (SDL2バックエンド)**:  
  * **描画 (hal\_video\_update)**: OSが生成したピクセル配列を、320×320のSDLテクスチャに書き込み、PC画面にレンダリングする。  
  * **入力 (hal\_input\_poll)**: PCキーボードのイベント（SDL\_Event）をフックし、PicoCalcのキーマトリクス構造体（上下左右、A/Bボタン等）にマッピングして返す。  
  * **音声 (hal\_audio\_callback)**: SDL\_Audioのコールバック関数を利用し、KotoOSのソフトウェアミキサーが生成したPCMバッファをPCのサウンドカードへ流し込む。  
  * **ファイル (hal\_fs\_read)**: PCのローカルディレクトリ（例: プロジェクトルートの ./sdcard\_mock/）をルートファイルシステムとしてマウントし、標準のstdio.hでラップする。  
* **実機用HAL (Pico SDKバックエンド)**:  
  * **描画**: DMAを初期化し、SRAM上のラインバッファをILI9488へSPI送信する。Dirty Rectangleの最適化を含む。  
  * **入力**: I2Cペリフェラル（100kHz設定）を叩き、STM32から送られてくるバイト列を解析してキー状態を返す。  
  * **音声**: ハードウェアタイマー割り込みを設定し、PWMスライスのデューティレジスタにミキサーのサンプリング値を書き込む。  
  * **ファイル**: SPI0を初期化し、FatFsを通じてSDカードのセクタを読み書きする。

### **9.2 アプリケーション実行方式の選択**

前述の通り、PicoCalcの実機（RP2040）において「SDカードから任意のネイティブアプリ（C/C++コンパイル済みバイナリ）をロードして実行する」ことは、SRAMの容量不足およびPSRAMの実行不可制約により極めて困難である。  
したがって、KotoRuntimeの実行アーキテクチャとしては「バイトコードVM方式」を強く推奨する。  
具体的な実装候補として、非常にフットプリントが小さいWebAssemblyインタプリタ（Wasm3など）や、軽量言語のVM（mruby、Lua、あるいはP/ECEライクな専用のスタックマシン）をC言語で自作しOSに組み込む。アプリケーション開発者はPC上でC言語やスクリプトを記述してバイトコードにコンパイルし、.kpa形式でパッキングする。実行時、KotoOSのVMはPSRAM上に置かれたバイトコードをシリアルに読み出しながら解釈・実行する。この方式であれば、SRAMを消費するのはVMの内部ステート（数十KB）のみとなり、無限に近いサイズのアプリケーションを安全（OSクラッシュを防ぐSandbox環境）に実行可能である。

### **9.3 MVP（Minimum Viable Product）実装ロードマップ**

実機到着前から開始できる、堅実な開発ステップを定義する。  
**フェーズ1: KotoSimの構築とコアAPIの確立（実機なし・PC環境のみ）**

* SDL2を用いたシミュレータ環境のセットアップ。  
* 画面描画API、フォントラスタライズ（美咲フォントの表示）、キー入力ポーリングのC言語インターフェースを定義。  
* モックのファイルシステムを用いて、KotoShell（ランチャーUI）のプロトタイプを作成。画面遷移のテスト。

**フェーズ2: 実機への移植とベアメタル駆動**

* Pico SDK環境の構築。フェーズ1のコードをPico向けにクロスコンパイル。  
* 実機HALの実装。まずは低速でも良いのでILI9488へのSPI描画を成功させる。  
* STM32からのI2Cキーボード入力の取得。10kHzバグの回避とレイテンシの検証 15。

**フェーズ3: 最適化とKotoOS基盤の完成**

* 描画ルーチンをDMA＋ラインバッファ方式に書き換え、FPSを向上させる。  
* FatFsを組み込み、SDカードからのリソース読み込みを実装。  
* SKK風のIMEロジック（Sticky Shift）を実装し、メモ帳アプリを完成させる。

**フェーズ4: アプリケーションエンジンと音響の統合**

* ソフトウェアPCMミキサーをPWM割り込みに統合し、効果音を鳴らす。  
* バイトコードVMを組み込み、KotoRuntimeを完成させる。  
* SDカード上のKotoDOS風ミニゲームやKotoVN（ビジュアルノベル）がランチャーから起動・実行できることを実証する。

## **10\. 比較分析：歴史的小型コンピューティング環境との対比**

PicoCalcを基盤とするKotoOSのコンセプトは、歴史的な小型デバイスや現代のメイカー向けガジェットと密接な関連を持つ。それぞれの環境とPicoCalcを比較し、KotoOSのポジショニングを明確にする。

| 比較環境                 | 画面・表示能力                       | 処理能力・メモリ                              | アプリ配布・開発                     | KotoOS (PicoCalc) との相性と継承点                                                                                                                                                                                                                                                 |
| :----------------------- | :----------------------------------- | :-------------------------------------------- | :----------------------------------- | :--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **P/ECE (ピース)** 27    | 128×88 ピクセル (4階調モノクロ)      | EPSON S1C33 (24MHz) 256KB SRAM                | USB連携 / ネイティブC言語開発        | **完全な精神的後継**。 CPU速度とカラー解像度は劇的に進化したが、中核となる内部SRAM容量（264KB）は奇しくもP/ECE（256KB）と同等である。当時の緻密なメモリ管理技術とリソース節約の哲学がそのまま活きる、理想的なキャンバスである。                                                    |
| **BrainPad TiPO** 29     | 液晶タッチパネル                     | 専用プロセッサ 数MBメモリ                     | BTRONアーキテクチャ、実身/仮身モデル | **OS UIの理想形**。 TiPOに見られる「画面下部の固定IME入力ライン」や、すべてのデータをカプセル化して扱うファイル管理の思想は、KotoIMEおよびKotoFSの設計に大いに借用すべき優れたパラダイムである。                                                                                   |
| **DOS/VGA 小型ゲーム**   | 320×200 ピクセル (256色インデックス) | i386等 (数MHz〜) 640KB RAM (コンベンショナル) | フロッピーディスク / HDD             | **移植対象として極めて有望**。 PicoCalcの320×320画面のうち、上部320×200をゲーム領域とすることで、当時のアスペクト比と描画アルゴリズム（Dirty Rectangle等）を完全に再現・エミュレート可能である。                                                                                   |
| **ポケコン (PC-G850等)** | 数行のキャラクタLCD                  | Z80等 / 数十KB RAM                            | カセットテープ / シリアル            | **インタラクティブ性の規範**。 PicoCalcの物理キーボードとPicoMite（BASIC）環境はポケコンの現代的再誕と言える。KotoOSにおいても、シェル環境から直接スクリプトを打ち込めるREPL機能の実装が期待される。                                                                               |
| **M5Stack Cardputer** 30 | 1.14インチ 240×135 (フルカラー)      | ESP32-S3 (240MHz) 8MB SRAM/PSRAM              | SDカード / MicroPython, Arduino      | **現代における強力なライバル機**。 ESP32-S3はSRAM容量と処理能力でPicoCalcを凌駕するが、画面解像度と視認性（特に複雑な漢字フォントを表示する際の明瞭さ）においては、4インチ 320×320を誇るPicoCalcが圧倒的に勝る。ビジュアルノベルやテキストエディタの構築にはPicoCalcが適している。 |
| **ClockworkPi uConsole** | 5インチ 1280×720                     | Raspberry Pi CM4等 数GB RAM                   | Linux OS / パッケージマネージャ      | **アプローチの対極**。 同じ製造元ながら、uConsoleは完全なLinux PCでありリソースの制約が実質存在しない。PicoCalc上のKotoOSは、リソースを極限まで絞り込み、ベアメタルでハードウェアを直接叩き切る「ハッカーの遊技場」としてのロマンを追求するベクトルにある。                        |

## **結論**

ClockworkPi PicoCalc上で「KotoOS」を構築するプロジェクトは、SPIディスプレイの帯域限界、内部SRAMの枯渇、PIO駆動PSRAMの特異性、そしてPWMオーディオの制約といった数々のハードウェア的な障壁を伴う。しかし、スキャンラインベースのDMA描画、ソフトウェアPCMミキサー、そして軽量バイトコードVMによるアプリケーション実行というソフトウェアアーキテクチャの工夫によって、これらの制約はすべて突破可能である。  
特に、PC上のSDL2シミュレータを利用して実機とコードを共通化するHAL分離アプローチを採用することで、開発効率は飛躍的に向上する。PicoCalcという制約に満ちた箱庭の中に、P/ECEやBTRONの思想を受け継いだモダンかつレトロなOS空間を築き上げることは、技術的探求として極めて実現性が高く、意義深い挑戦である。さらに、コアモジュールをRP2350搭載のPico 2へ換装することで、QSPI PSRAMのマッピングや大幅な性能向上が見込めるという、将来的な発展性も担保された堅牢なプラットフォームであると結論付ける。

#### **引用文献**

1. GitHub \- clockworkpi/PicoCalc: A calculator like nothing you've ever seen before, right?, 6月 13, 2026にアクセス、 [https://github.com/clockworkpi/PicoCalc](https://github.com/clockworkpi/PicoCalc)  
2. PicoCalc \- ClockworkPi, 6月 13, 2026にアクセス、 [https://www.clockworkpi.com/picocalc](https://www.clockworkpi.com/picocalc)  
3. PicoCalc kit \- ClockworkPi, 6月 13, 2026にアクセス、 [https://www.clockworkpi.com/product-page/picocalc](https://www.clockworkpi.com/product-page/picocalc)  
4. PSRAM on the PicoCalc \- PicoCalc \- clockworkpi, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/psram-on-the-picocalc/17176](https://forum.clockworkpi.com/t/psram-on-the-picocalc/17176)  
5. New LCD screen (ST7365P) in recent PicoCalc commit? \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/new-lcd-screen-st7365p-in-recent-picocalc-commit/17649](https://forum.clockworkpi.com/t/new-lcd-screen-st7365p-in-recent-picocalc-commit/17649)  
6. GitHub \- EngineerDogIta/picocalc\_micropython\_project: This project ..., 6月 13, 2026にアクセス、 [https://github.com/EngineerDogIta/picocalc\_micropython\_project](https://github.com/EngineerDogIta/picocalc_micropython_project)  
7. Audio with PIO PWM \- PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/audio-with-pio-pwm/18339](https://forum.clockworkpi.com/t/audio-with-pio-pwm/18339)  
8. Gpio for Pico calc, how to make firmware for Pico calc \- PicoCalc ..., 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/gpio-for-pico-calc-how-to-make-firmware-for-pico-calc/20905](https://forum.clockworkpi.com/t/gpio-for-pico-calc-how-to-make-firmware-for-pico-calc/20905)  
9. Reading battery level from mmbasic \- PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/reading-battery-level-from-mmbasic/16433](https://forum.clockworkpi.com/t/reading-battery-level-from-mmbasic/16433)  
10. PicoCalc SD cards cant be read \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/picocalc-sd-cards-cant-be-read/20843](https://forum.clockworkpi.com/t/picocalc-sd-cards-cant-be-read/20843)  
11. LCD ILI9488 really doesn't support 565 RGB mode? \- PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/lcd-ili9488-really-doesnt-support-565-rgb-mode/16573](https://forum.clockworkpi.com/t/lcd-ili9488-really-doesnt-support-565-rgb-mode/16573)  
12. Ideas for improving LCD speed \- PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/ideas-for-improving-lcd-speed/17159](https://forum.clockworkpi.com/t/ideas-for-improving-lcd-speed/17159)  
13. Ideas for improving LCD speed \- Page 2 \- PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/ideas-for-improving-lcd-speed/17159?page=2](https://forum.clockworkpi.com/t/ideas-for-improving-lcd-speed/17159?page=2)  
14. Macintosh emulator v0.06b \- cmd key \- PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/macintosh-emulator-v0-06b-cmd-key/17374](https://forum.clockworkpi.com/t/macintosh-emulator-v0-06b-cmd-key/17374)  
15. I2C / Keyboard Speed \- PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/i2c-keyboard-speed/21923](https://forum.clockworkpi.com/t/i2c-keyboard-speed/21923)  
16. SD card access shenanigans \- PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/sd-card-access-shenanigans/22293](https://forum.clockworkpi.com/t/sd-card-access-shenanigans/22293)  
17. Where can I find information on the PicoCalc SD card slot? \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/where-can-i-find-information-on-the-picocalc-sd-card-slot/17953](https://forum.clockworkpi.com/t/where-can-i-find-information-on-the-picocalc-sd-card-slot/17953)  
18. A header-only C library to allow access to SPI PSRAM via PIO on the RP2040 microcontroller. \- GitHub, 6月 13, 2026にアクセス、 [https://github.com/polpo/rp2040-psram](https://github.com/polpo/rp2040-psram)  
19. reading battery status via i2c always reports 0 · Issue \#20 · clockworkpi/PicoCalc \- GitHub, 6月 13, 2026にアクセス、 [https://github.com/clockworkpi/PicoCalc/issues/20](https://github.com/clockworkpi/PicoCalc/issues/20)  
20. Fonts for PicoMite on the PicoCalc \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/fonts-for-picomite-on-the-picocalc/18029](https://forum.clockworkpi.com/t/fonts-for-picomite-on-the-picocalc/18029)  
21. Basic programs the PicoCalc \- Page 3 \- ClockworkPi Forum, 6月 13, 2026にアクセス、 [https://forum.clockworkpi.com/t/basic-programs-the-picocalc/17822?page=3](https://forum.clockworkpi.com/t/basic-programs-the-picocalc/17822?page=3)  
22. Releases · clockworkpi/PicoCalc \- GitHub, 6月 13, 2026にアクセス、 [https://github.com/clockworkpi/PicoCalc/releases](https://github.com/clockworkpi/PicoCalc/releases)  
23. PicoCalc/Code/FUZIX/README.md at master \- GitHub, 6月 13, 2026にアクセス、 [https://github.com/clockworkpi/PicoCalc/blob/master/Code/FUZIX/README.md](https://github.com/clockworkpi/PicoCalc/blob/master/Code/FUZIX/README.md)  
24. PicoMite User Manual \- Geoff's Projects, 6月 13, 2026にアクセス、 [https://geoffg.net/Downloads/picomite/PicoMite\_User\_Manual.pdf](https://geoffg.net/Downloads/picomite/PicoMite_User_Manual.pdf)  
25. Technical Specifications \- Calculinux, 6月 13, 2026にアクセス、 [https://calculinux.org/hardware/specifications/](https://calculinux.org/hardware/specifications/)  
26. PicoCalc Out-of-Box Tutorial: Assembly, BASIC, GPIO & Firmware Fun\! \- YouTube, 6月 13, 2026にアクセス、 [https://www.youtube.com/watch?v=ATwZwTEeSPo](https://www.youtube.com/watch?v=ATwZwTEeSPo)  
27. P/ECE \- Aquaplus Wiki \- Fandom, 6月 13, 2026にアクセス、 [https://aquaplus.fandom.com/wiki/P/ECE](https://aquaplus.fandom.com/wiki/P/ECE)  
28. Aquaplus P/ECE (vs Panic Playdate) I Get Info \- Matt Sephton, 6月 13, 2026にアクセス、 [https://blog.gingerbeardman.com/2021/08/19/aquaplus-piece-vs-panic-playdate/](https://blog.gingerbeardman.com/2021/08/19/aquaplus-piece-vs-panic-playdate/)  
29. BTRON \- Wikipedia, 6月 13, 2026にアクセス、 [https://en.wikipedia.org/wiki/BTRON](https://en.wikipedia.org/wiki/BTRON)  
30. M5Stack Cardputer Adv (ESP32-S3) \- RobotShop, 6月 13, 2026にアクセス、 [https://www.robotshop.com/products/m5stack-cardputer-adv-esp32-s3](https://www.robotshop.com/products/m5stack-cardputer-adv-esp32-s3)
