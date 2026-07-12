# **PicoCalc-class RP2040向け 純Rust製 PSRAM-backed グラフィックス基盤「KotoGFX」要件定義・技術調査・設計方針レポート**

## **Executive Summary**

本レポートは、PicoCalcクラスのRP2040搭載デバイスに向けた、Pure RustベースのPSRAM駆動型2Dグラフィックスおよびゲームフレームワーク（仮称：KotoGFX）の構想について、技術的妥当性、要求仕様、アーキテクチャ設計、およびリスク分析を包括的にまとめた専門的調査である。本構想の核心は、SRAM容量が264KBと極めて限られるRP2040環境において、大容量の外部QPI PSRAM（約8MB）を描画状態の保持、アセットの格納、およびオフスクリーン領域として最大限活用する点にある。同時に、フルフレームバッファをSRAMに配置する従来の手法を放棄し、LVGLのアーキテクチャから着想を得たRetained Rendering（保持型レンダリング）、Dirty Area Invalidation（変更領域の無効化）、およびPartial Flush（部分転送）の思想を組み込み用途に最適化して導入する。  
技術的調査および分析の結果、本構想は十分に成立可能であると結論付けられる。RP2040が備えるProgrammable I/O (PIO) とDirect Memory Access (DMA) の非同期並列処理能力を駆使することで、SRAM上には数キロバイトの小さなラインバッファ（チャンクバッファ）を保持するだけで、実用的なフレームレートでの画面更新が可能である1。この設計により、KotoOSなどのホストオペレーティングシステムや仮想マシン（VM）に対して十分なSRAM空間を残すことができる。  
最大の技術リスクは、PSRAMに対するQPI（Quad Peripheral Interface）PIOアクセスの安定性と、LCDへのSPI転送時におけるシステムバスの競合（DMA Contention）に起因するレイテンシである3。特にRP2040の内部バスアーキテクチャにおいて、命令フェッチや他のペリフェラルによるアクセスがDMA転送をストールさせる可能性が懸念される。これらのリスクを低減するためには、クリティカルなレンダリングコードのSRAM配置や、ストライプ化されたメモリバンクの戦略的利用が不可欠となる。  
最小実装（MVP）の推奨スコープとしては、まずはハードウェアドライバ層（PSRAMおよびLCD）の完全な分離と安定化を図り、次に16x16ピクセルを基本とするTile Layerを用いたチャンクベースの合成エンジンを確立することを推奨する。高度な汎用GUIフレームワークを目指すのではなく、Shell、Memo、Launcher、および小型ゲームの動作に特化した三層構造（Tile, Sprite, Text）の実装に注力することが、本プロジェクトの成功の鍵となる。

## **Requirements**

本セクションでは、KotoGFX構想を実現するために必要な機能的・非機能的要件、およびハードウェア、メモリ、パフォーマンスの各制約を定義する。

### **Functional Requirements**

KotoGFXは、アプリケーションが毎フレームすべての描画コマンドを発行する即時描画（Immediate Mode）ではなく、描画状態をフレームワーク側で保持するRetained Modeを中心とした機能を提供する。必須となる機能要件は、Tile Layer、Sprite Layer、Text Layerからなる三層の描画構造を確立することである。Tile Layerは背景や固定UIを管理し、16x16ピクセルのセルベースで動作する。Sprite Layerはキャラクターや動的オブジェクトを担当し、位置の移動に伴って自動的に無効化領域（Old RectとNew Rect）を計算しダーティリストに登録する機能を持つ。Text LayerはグリフIDベースの文字セルレイヤーとして機能し、HUDやシェル画面などのテキスト情報を描画する。  
また、状態が変化した矩形（Dirty Rect）を追跡し、これらをマージ、クリップ、およびタイル境界にアライメントして最小の再描画領域を算出するダーティトラッカーの実装が求められる。さらに、PicoCalcのキーボード入力等を処理するための、イベント駆動型の軽量な入力ディスパッチ機構を内包し、2Dゲーム向けAPIと共通化することが求められる。

### **Non-functional Requirements**

本フレームワークは、Rustのno\_std環境を基本とするPure Rust実装でなければならない。OSやVMからホストコールとして扱いやすいシンプルなAPI設計とし、動的メモリ確保（ヒープアロケーション）への依存を極限まで排除する。先行実装であるPicowareの構成やハードウェアの知見は参考にするものの、GPL-3.0等で公開されている既存のC/C++コードの直接移植は一切行わず、クリーンルーム設計による独自のアーキテクチャを採用することで、ライセンス面での完全な独立性を担保する5。

### **Hardware Requirements**

ターゲットデバイスは、RP2040（デュアルコア Cortex-M0+, 133MHz〜）を搭載したPicoCalcクラスのハードウェアである。ディスプレイとしては、320x320ピクセル解像度を持つカラーLCD（ILI9488P等）をSPI経由で駆動することを想定する。また、外部メモリとしてGPIOに接続されたQPI駆動のPSRAM（容量約8MB）を必須とする。PicoCalcの実機においては、LCD SPI（GP10〜12）や、STM32ベースのキーボードモジュールと通信するためのI2C（GP6、GP7）といった特定のピン配置が存在するため、これらのハードウェアリソースを効率的に制御する構成が求められる7。

### **Memory Requirements**

SRAM容量の厳格な管理が本構想の最重要課題である。RP2040の264KBのSRAMは、VM、ネットワークスタック、オーディオバッファ、その他のシステム機能と共有される。したがって、320x320解像度のRGB565フルフレームバッファ（約200KB）をSRAMに配置することは物理的にも論理的にも不可能である。SRAMには、描画中の数行分のピクセルを保持するラインバッファ（チャンクバッファ）、ダーティ矩形のリスト、およびアクティブなスプライトのメタデータキャッシュといった、極めて小さな領域のみを配置する。描画状態の実体や画像アセット、オフスクリーン領域はすべて外部のPSRAM側に格納し、必要なタイミングでフェッチするアーキテクチャとする。

### **Performance Requirements**

RP2040のリソース制約下において、軽量かつ滑らかな動作を実現することが目標である。小さなダーティ領域のみを更新するケース（例：カーソル移動やテキスト入力）では、部分的な再描画とLCDへのPartial Flushにより60FPSでの高速な応答性を維持する。一方、画面全体のスクロールなど全領域の更新が必要な場合においても、フォールバック機構を用いて最低30FPSのフレームレートを確保する。これを達成するためには、CPU処理をブロックすることなくデータの移動を行うDMAとPIOの統合が必須となる1。

### **API Requirements**

APIは、VMのホストコールとして効率的にマッピングできる粒度で設計される必要がある。アプリケーション側からは、tile\_set\_cellやsprite\_moveといった状態変更命令のみを発行し、最終的にpresentを呼び出すことでフレームワーク側が一括して描画を処理する構造とする。既存の汎用グラフィックスライブラリに見られるような即時描画API（draw\_rect等）は、システムパニック時のフォールバックやデバッグオーバーレイ用途などの例外的な機能として分離し、メインのレンダーパイプラインには混入させない。

### **Safety / Reliability Requirements**

Rustの型システムと所有権モデルを最大限に活用し、安全性と信頼性を確保する。PSRAM空間はKotoGFXが専有するのではなく、OSやランタイムから明確に定義された境界（Region）として割り当てられる。ポインタの直接操作を避け、インデックスベースのアセットハンドルや厳格な境界チェックを伴うアクセス抽象化層を導入することで、領域外参照やメモリ破壊をコンパイル時および実行時に防止する。

## **Architecture Proposal**

KotoGFXのアーキテクチャは、関心の分離を徹底し、依存関係を明確にするために複数のRustクレートに分割して構成される。このモジュール分割は、保守性やテスト容易性を高め、各層の独立した最適化を可能にする。

### **module/crate構成**

クレートの境界線は、ハードウェアの制御、グラフィックスの論理演算、そして上位のアプリケーションインターフェースという三つの大きなドメインに従って引かれる。以下の5つの主要クレートに分割する案は、組み込みRustのベストプラクティスに合致しており極めて妥当である。

#### **PSRAM driver層 (koto-psram)**

この層は、QPI PSRAMとの低レベルな通信を担う。RP2040のPIOを用いたクロックサイクル単位での精密なタイミング制御、ダミーサイクルの挿入、およびDMA転送の設定をカプセル化する3。グラフィックス関連のロジックは一切含めず、純粋なメモリアクセスバスとして機能する。テストスイートや診断モジュール（周波数のスイープテストやクロック分周比のチューニング）は、本番用APIとは明確に分離して実装する。

#### **display flush層 (koto-display)**

LCDへのデータ転送を専門に行うドライバ層である。指定された矩形領域のウィンドウをLCDコントローラに設定し、DMAを介してSRAM上のチャンクバッファからRGB565データを非同期にSPI送信する役割を持つ。Partial Flushの実装において、転送完了割り込みやステータスフラグの管理を行い、CPUが次のチャンクの合成を並行して実行できるよう非同期インターフェースを提供する9。

#### **gfx core層 (koto-gfx)**

KotoGFXの心臓部であり、すべての論理的な描画状態を管理する。色表現、矩形計算、ダーティトラッカー、Tile/Sprite/Textの各レイヤー状態の保持、そして合成器（Compositor）を内包する。この層はハードウェアに依存しない純粋な論理層として構築され、koto-psramから提供されるトレイトインターフェースを介してPSRAM上のストレージ領域（PsramRegion）を操作する。

#### **game2d API層 (koto-game2d)**

koto-gfxの上に構築され、2Dゲームやインタラクティブなアプリケーションの構築に必要なドメイン固有のAPIを提供する。入力状態の管理やスプライトアニメーションの抽象化を行い、VMが解釈しやすいホストコールへのマッピングを担当する。特定のゲーム（例：KotoBlocks専用のdraw\_piece）に特化した命令は避け、あらゆる2Dグリッドゲームに適用可能な汎用性を持たせる。

#### **optional UI層 (koto-ui)**

Shell、Memo、Launcherなどのシステムアプリケーションを構築するための軽量なウィジェットレイヤーである。ラベル、ボタン、リストなどの基本コンポーネントを提供するが、LVGLのような複雑なウィジェットツリーや動的レイアウト計算（Flexbox等）は持たない。絶対座標指定によるフラットな配置を基本とする。

#### **VM hostcall層**

KotoOSの仮想マシンとネイティブのKotoGFX基盤を接続するインターフェース層である。引数のシリアライズ・デシリアライズやエラーハンドリングを集約し、VMの命令セットから安全かつ低負荷でグラフィックスAPIを呼び出せるよう調停する。

## **Memory Model**

メモリ管理は、限られたリソースを最適に分配するための最も重要な設計要素である。PSRAMはグラフィックス専有ではなく、システム全体のリソースプールとして共有される。

### **PSRAM全体配分案**

8MBのQPI PSRAMは、OSレベルの固定配置方式または静的アリーナアロケータによって領域分割される。典型的な分配設計は以下のようになる。

| 領域名 | 容量目安 | 用途と特性 |
| :---- | :---- | :---- |
| System / Allocator | 256 KB | OSのメタデータ、グローバルなメモリアロケータの管理領域。 |
| App Bytecode / VM | 1 MB | アプリケーションのバイトコード、VMの実行キャッシュ、インタープリタの状態。 |
| File / Resource Cache | 2 MB | ファイルシステムのキャッシュ、展開済みのデータ、各種リソースの一時保存。 |
| **KotoGFX Working Store** | **1 MB** | **描画状態（タイルマップ配列、スプライト定義、文字セル配列）およびダーティリストの保持。** |
| **Graphics Assets** | **2 MB** | **スプライトシート、タイルセットのピクセルデータ、フォントのグリフキャッシュ。** |
| Audio Buffer | 512 KB | オーディオのPCMデータやストリーミング再生用のリングバッファ10。 |
| Optional Framebuffer | 256 KB | 疑似フルスクリーンダブルバッファ（デバッグ用や特殊なフォールバック用）。 |
| Scratch / Reserve | 約1 MB | 診断ログ、一時的なスクラッチメモリ、および将来の拡張用リザーブ領域。 |

### **KotoGFX region設計**

KotoGFXには、PSRAM全体の管理権限は与えられず、OSから指定された領域情報を受け取る。設計としては簡易アロケータ方式ではなく、固定配置の境界アドレスを渡す固定配置方式が絶対的に有利である。動的なメモリ割り当ては断片化を引き起こし、組み込みシステムにおける長時間稼働時のクラッシュ原因となるためである。  
システムからは以下のような構造体として領域が引き渡される。

Rust  
pub struct PsramRegion {  
    pub base\_addr: u32,  
    pub capacity: u32,  
}

KotoGFXは内部でこの領域をgfx\_work\_region、gfx\_asset\_region、glyph\_cache\_regionの3つの論理サブ領域に静的に分割し、インデックスを用いたオフセット計算によって各データ構造にアクセスする。

### **SRAM使用量見積もり**

SRAMの使用量はシステム全体の安定性に直結するため、極限まで切り詰める必要がある。KotoGFXの動作に必要なSRAM予算の目安は以下の通りである。

* **最小構成 (Minimal Budget)**: 合成用チャンクバッファを 320 x 8 ピクセルとし、ダーティ矩形リストを最大16個に制限。必要なSRAMは約 6KB。タイルグラフィックスの局所的な色変換キャッシュ等に 2KB を追加し、合計約 8KB。  
* **標準構成 (Standard Budget)**: チャンクバッファをタイルサイズに合わせた 320 x 16 ピクセルのダブルバッファリング構成（10KB x 2 \= 20KB）とする。ダーティ矩形リストを32個、スプライトメタデータキャッシュを追加し、合計約 24KB。  
* **豪華構成 (Luxury Budget)**: より大きなチャンクバッファ（例：320 x 32ピクセル）や、よく使う文字グリフのSRAM上への投機的キャッシングを行い、合計約 48KB。

現実的な運用としては、パフォーマンスとメモリ消費のバランスが最も優れた標準構成（約24KB）をデフォルトとすることが望ましい。

## **Rendering Pipeline**

SRAMにフルフレームバッファを置かないため、レンダリングパイプラインは「状態の変更追跡」と「チャンク単位の部分合成」の二つのフェーズからなる。

### **dirty rect flow**

アプリケーションが描画状態を変更するAPI（tile\_set\_cellやsprite\_set）を呼び出すと、変更の影響を受ける矩形領域がダーティトラッカーに登録される。スプライトが移動した場合、以前の位置（Old Rect）を無効化して背景を描画し直し、新しい位置（New Rect）も無効化してスプライトを描画し直す必要があるため、2つの矩形が登録される。テキストセルの変更時も、該当する文字のバウンディングボックスが登録される。  
登録された矩形は、交差テストによって重なり合うものが結合（Merge）される。RP2040の計算能力を考慮すると、矩形の数が過大になると結合処理自体のコストが跳ね上がるため、リストの最大数は32程度に制限する。さらに、合成処理を単純化しメモリアクセスの境界を揃えるため、結合された矩形は16x16ピクセルのタイル境界にアライメントされる。

### **chunk compositor**

画面全体を特定の高さ（例：16ピクセル）のバンド状の「チャンク」に分割して合成処理を行う。ダーティ矩形が全く存在しないチャンクは完全にスキップされる。チャンクバッファへの合成は、以下の三層の順でピクセルを書き込んでいく。

1. **tile rendering**: 最下層として背景を描画する。PSRAMから対象チャンクに含まれるタイルIDの配列を読み出し、対応するRGB565のピクセルデータをSRAMのチャンクバッファへ順次転送する。  
2. **sprite rendering**: 対象チャンクと交差するスプライトを抽出し、オーバードローを行う。特定の色（カラーキー）を透明として扱う条件分岐を挟みながら、タイルグラフィックスの上にピクセルを上書きしていく。  
3. **text rendering**: 最後に、テキストセル層のデータを評価する。1bppのグリフデータをPSRAMのフォントキャッシュから読み出し、前景色と背景色に展開しながらビットマスク処理によってチャンクバッファへ書き込む。

### **partial LCD flush**

一つのチャンクの合成がSRAM上で完了すると、DMAに対してSPI送信のトリガーを発行する。設定されたウィンドウ領域に対し、SRAMのチャンクバッファからLCDのGRAMへRGB565データが非同期で流し込まれる。このデータ転送中、CPUは次のチャンクの合成を裏のバッファ（ダブルバッファ）で行う。このパイプライン化により、SPIの転送待ち時間を実質的にゼロに隠蔽することができる8。

### **fallback full redraw**

ダーティ矩形の数が制限を超過するか、結合された矩形の総面積が画面全体の一定割合（例えば70%）を超過した場合、ダーティ領域ごとの部分更新を行うオーバーヘッドが全再描画のコストを上回る。この状況を検知した場合、システムはダーティトラッキングを放棄し、画面最上部から最下部までチャンク単位で全再描画を行うフォールバックモードへと切り替わる。この全再描画時において、RP2040とSPI LCDの帯域限界から想定されるフレームレートは約30FPSとなる。

## **API Design**

KotoGFXのAPI設計は、アプリケーションが描画ピクセルを直接操作するのではなく、フレームワークが管理するデータ構造の「状態を宣言・変更」する形をとる。

### **Rust API案**

RustのAPIはモジュール化された安全な関数群として提供される。以下は中核となるインターフェースの概念である。

Rust  
// レイヤー定義と更新  
pub fn tile\_define(region: \&PsramRegion, tile\_id: u16, data: &\[u8\]) \-\> Result\<(), Error\>;  
pub fn tile\_set\_cell(x: u8, y: u8, tile\_id: u16) \-\> Result\<(), Error\>;

// スプライトの割り当てと操作  
pub fn sprite\_alloc() \-\> Result\<SpriteId, Error\>;  
pub fn sprite\_set(id: SpriteId, x: i16, y: i16, tile\_id: u16) \-\> Result\<(), Error\>;  
pub fn sprite\_hide(id: SpriteId) \-\> Result\<(), Error\>;

// テキストと最終描画コマンド  
pub fn text\_put\_cell(x: u8, y: u8, glyph\_id: u16, fg: Color, bg: Color) \-\> Result\<(), Error\>;  
pub fn present() \-\> Result\<FrameStats, Error\>;

エラーハンドリングは、組み込みシステムにおけるパニックを避けるため、一貫してResult型を返す方針とする。

### **VM hostcall案**

KotoOS上のVMから呼び出されるAPIは、VMの命令フェッチやコンテキストスイッチのオーバーヘッドを最小化するため、さらに粒度の粗い設計とする。細かいセル単位の更新をループで呼び出すのではなく、バッチ処理を前提とする。

* hostcall\_tile\_fill\_rect(x, y, w, h, tile\_id)  
* hostcall\_sprite\_move\_bulk(ptr\_to\_array\_of\_updates, count)  
* hostcall\_text\_write\_line(x, y, ptr\_to\_glyph\_ids, count)

これにより、VM命令の実行回数とホストコール呼び出し回数が激減し、システム全体のパフォーマンスが大きく向上する。

### **immediate APIとの関係**

draw\_rectやdraw\_pixels\_rgb565のようなImmediate（即時描画）APIは、Retained Renderingを基本とするKotoGFXの標準パイプラインと競合するため、メインのAPI群からは排除するべきである。これらの機能は、システムクラッシュ時のブルースクリーン描画や、オーバーレイデバッグ用途に特化した独立モジュールとして扱い、通常のアプリケーション開発には使用させない設計とする。

### **budgeted immediate overlay model（次の実装ステップ）**

Retained を基本としつつも、KotoSnake のような小型ゲームには即時描画でしか自然に表現できない演出が残る——流れる虹色のスネーク胴体、エサ取得時のパーティクル、フラッシュ、ポップアップ、その他の一過性オーバーレイである。これらを retained タイルマップ層へ無理に押し込むとゲームフィールが変わってしまうため、即時描画パスとして残す。一方で、即時コマンドが無制限に増えると `MAX_APP_DRAW_COMMANDS` を溢れさせ、末尾コマンドの暗黙のドロップ（tail-drop）とそれに伴うフルリペイントを誘発する。現行の上限は先着順（first-come-first-served）でしかなく、アプリが最後に積んだ——多くの場合最も見せたい頭・エサ・スコアの——コマンドが落ちる。

この問題に対し、KotoGFX に純粋な（`no_std`・ヒープ非依存・依存ゼロの）予算ポリシー層を導入する（`koto-gfx` の `budget` モジュール）。即時描画を [`DrawClass`]（CoreGameplay / Actor / CriticalUi / Particles / Decoration / Debug）でタグ付けし、各クラスに [`OverlayPriority`] と、[`DrawBudget`] 上の保証予約（reservation）と共有プールの保護フロア（shared-pool floor）を与える。採否判定 [`DrawBudget::request`] は、まずそのクラス専用の予約から、次に共有プールから（より重要な後着クラスのための余地を残しつつ）コマンドを割り当て、結果を [`BudgetDecision`]（Admit / Degrade / Reject）として返す。会計 [`BudgetStats`] は構造上、設定された上限（cap）を決して超えない。これにより、重要な描画は——発行順に関わらず予約によって席が確保され——常に通り、装飾的な演出は cap に達する前（＝ tail-drop とフルリペイントが起きる前）に degrade / reject される。

現時点ではこれは**ポリシーデータのみ**であり、いかなるアプリの描画パスにもまだ接続されていない（visual は不変）。共有しているのは cap の*値*だけで、`MAX_APP_DRAW_COMMANDS` は散在する裸の定数ではなく `koto_gfx::APP_DRAW_BUDGET`（その予約がサイズ合わせの基準とする値）から導出される。

**観測モード（次の実装ステップ・実装済み）**: 描画を一切ゲートしないまま、予算モデルが*何を判定したか*だけを記録する診断モードを追加した。KotoSnake の即時描画をホスト側で [`DrawClass`] に分類し（バイトコード・hostcall・ABI は不変）、`APP_DRAW_BUDGET` に対して採否を計測する（`src/koto-sim/tests/fixture_runner.rs` の `kotosnake_immediate_overlay_budget_observation` / `kotosnake_worst_case_long_snake_budget_pressure`）。観測結果は [KOTO_KOTOSNAKE_BUDGET_OBSERVATION.md](../devlog/KOTO_KOTOSNAKE_BUDGET_OBSERVATION.md) に記録した。要点: KotoSnake は盤面を retained static 層、HUD を retained text に逃がしているため即時パスに CoreGameplay / CriticalUi が存在せず、それらに割り当てた 28 コマンドの予約が遊休（stranded）となって共有プールを圧迫する。その結果、長いスネーク＋エサ取得バーストの最悪ケースでは Particles が degrade、後着の Decoration が reject されるが——いずれも cap（96）に達する*前*に起こる（＝ tail-drop とフルリペイントの前に演出を間引く、という狙い通りの挙動）。レンダリング出力は不変。

次のステップは、この予算モデルを実際の即時オーバーレイ描画に段階的に適用し、retained 層と予算化された即時描画を完全に分離することである（その際、観測で得た数値を使って KotoSnake 型アプリ向けに予約レイアウトを right-size できる）。

## **Data Structures**

SRAMの消費を抑えつつPSRAMへのアクセス効率を最大化するため、データ構造は極めてコンパクトにパックされたバイナリ形式で定義される。

* **TileCell**: 単一の u16 型で表現する。下位12ビットをタイルID（最大4096種類）、上位4ビットを回転や反転などのフラグに割り当てる。320x320解像度で16x16ピクセルのタイルを使用する場合、画面全体（20x20セル \= 400セル）はわずか800バイトで表現可能である。  
* **Sprite**: アクティブなスプライトのメタデータはSRAM上にキャッシュされる。構造体は struct Sprite { x: i16, y: i16, tile\_id: u16, flags: u16 } の8バイト構成とする。最大64個のスプライトを管理しても512バイトしか消費しない。  
* **TextCell**: struct TextCell { glyph\_id: u16, fg: Color, bg: Color } の構成。Unicodeスカラー値を直接保持しない理由は、日本語等の多言語対応時にフォントレンダリングのルックアップオーバーヘッドを避けるためである。文字列はVM側で描画前にグリフIDの配列に変換されてからKotoGFXに渡される。  
* **Rect**: struct Rect { x1: i16, y1: i16, x2: i16, y2: i16 }。  
* **DirtyTracker**: 最大32要素の Rect の固定長配列と、現在登録されている数を管理するカウンターで構成される。複雑なR-Tree等は用いず、単純な線形探索による結合判定を行う。  
* **FrameStats**: 処理に要した時間、登録されたダーティ矩形数、スキップされたチャンク数などのプロファイリング情報を保持する構造体。

## **Format Design**

グラフィックスフォーマットの選定は、メモリ帯域とSRAMにおけるデコードコストのトレードオフを決定づける。

### **MVP段階での推奨フォーマット**

MVP（Minimum Viable Product）実装においては、すべてのグラフィックスアセットを **RGB565（16bpp）** ベースで設計することを強く推奨する。4bpp indexedカラーを採用した場合、PSRAMからの読み出しデータ量は4分の1になるものの、SRAM上でのパレット展開処理がコンポジタに加わり、論理的な複雑さが飛躍的に増大する。初期段階ではシステム全体のパイプライン確立とデバッグの容易さを優先し、RGB565を用いて動作を安定させるべきである。

### **Transparency（透過処理）**

スプライトの透過処理については、アルファブレンドのような複雑な演算は完全に排除する。RGB565の特定のピクセル値（例えばマゼンタ 0xF81F または黒 0x0000）を透過カラーキーとして設定し、合成時に一致した場合はチャンクバッファへの書き込みをスキップするシンプルなマスク処理を採用する。

### **4bpp indexed の将来的な評価**

最適化フェーズにおいては、TileとSpriteのフォーマットとして4bpp indexedの導入を検討する。RP2040のCortex-M0+はビットシフトおよびマスク演算において高い効率を発揮するため、PSRAMの転送帯域を節約しつつ、SRAM上でパレットルックアップを行いながらチャンクへ展開する処理は十分に高速化可能である。フォーマット拡張を見据え、描画コアはカラーピクセルを展開するトレイトインターフェースを通してアセットにアクセスする設計とする。テキストグリフについては、容量効率を最大化するため常に1bppのビットマップフォーマットを採用する。

## **Performance Analysis**

### **PSRAM read/write帯域の論点**

PSRAMに対するアクセス帯域は、KotoGFXの命脈を握る。C言語によるrp2040-psramライブラリの先行研究によれば、RP2040のPIOを使用し、クロックの立ち下がりエッジでデータをサンプリングする精密なタイミング制御を行うことで、標準クロック時でも極めて高速なQPI転送が可能であることが示証されている3。この帯域は、1フレームあたりに数キロバイトのデータをオンデマンドでフェッチするKotoGFXの要件を十分に満たす。

### **LCD flush帯域の論点**

LCD（ILI9488等）へのSPI送信は、最も時間のかかるプロセスである。DMAを介して非同期に転送を行うことでCPUを解放するが、LCDコントローラ自体の受信許容クロック上限（通常数十MHz）がボトルネックとなる。ダブルバッファリングを用いたPartial Flushによって待ち時間を隠蔽するアプローチが不可欠である。

### **ダーティ矩形数と合成コスト**

ダーティ矩形の数が増加すると、交差判定および結合にかかる計算コストが指数関数的に増大する。RP2040の演算能力では、32個を超える矩形リストの管理はフレームレートの低下を招く。したがって、近接する小さな矩形は早期に包含バウンディングボックスとして単一化するヒューリスティックなマージ戦略が必要である。

### **想定負荷とアプリ別推定**

* **KotoBlocks / Snake / Mines**: ゲーム盤面の一部（ブロックの落下やキャラクターの移動）のみが変化するため、ダーティ領域は全画面の10%未満に収まる。PIOとDMAによるフェッチ・転送が軽快に機能し、安定して60FPSを維持できる。  
* **Shell / Memo / Launcher**: テキストのタイピングやカーソル移動のシナリオでは極めて低負荷である。しかし、画面全体がスクロールする瞬間はダーティ領域が100%となるため、フォールバック機構が作動し、約30FPSでの全再描画にシフトする。スクロールが停止すれば即座に部分更新モードに復帰する。

## **Implementation Roadmap**

開発はリスクの高い低レイヤーから段階的に積み上げる8つのフェーズで進行する。

1. **Phase 1: PSRAM driver isolation** (koto-psram) PicowareのC++実装や既存のPIOアセンブリを参考にしつつ、純粋なRust環境下でQPI PSRAMドライバを独立して構築し、読み書きの帯域テストを完了させる3。  
2. **Phase 2: LCD partial flush** (koto-display)  
   LCDの初期化と、指定された矩形ウィンドウに対するDMA経由での非同期SPIピクセル転送機構を確立する。  
3. **Phase 3: Tile Layer MVP** (koto-gfx)  
   PSRAMからRGB565の画像データを読み出し、SRAMのチャンクバッファにタイルとして敷き詰めるコンポジタの初期バージョンを実装する。  
4. **Phase 4: Sprite Layer**  
   カラーキーによる透過合成の実装と、位置変更に伴うダーティ領域（Old/New Rect）の自動トラッキング機能を導入する。  
5. **Phase 5: Text Layer**  
   1bppグリフデータのパッキング解除と、色展開によるテキストレンダリング機能を合成器に統合する。  
6. **Phase 6: Game2D hostcalls** (koto-game2d)  
   KotoOSのVMから各種レイヤーを操作するためのバッチ処理型ホストコールAPIを定義し、仮想マシンとの結合を図る。  
7. **Phase 7: UI widgets** (koto-ui)  
   ShellやMemoアプリ向けに、絶対座標指定によるシンプルなラベル、ボタン、テキスト入力ウィジェットを構築する。  
8. **Phase 8: Optimization**  
   4bppパレットカラーへの対応、SRAM上でのインナーループの高度な最適化、およびDMAチャネルの優先度調整によるバス競合の緩和を実施する。

## **Risk Register**

技術的およびプロジェクト運営上のリスクを以下に定義し、その緩和策を提示する。

| リスク要因 | 影響度 | 確率 | リスクの詳細と緩和策 |
| :---- | :---- | :---- | :---- |
| **PSRAM QPIの安定性** | 致命的 | 中 | 高速動作時のクロック位相ずれや配線容量によるデータ化け。緩和策：クロック立ち下がりサンプリング等のPIO仕様を厳密に実装し、初期化時にタイミングキャリブレーションを行う3。 |
| **DMAバスの競合 (Contention)** | 高 | 高 | Flash（XIP）キャッシュミスや他ペリフェラルのアクセスがDMAをストールさせる問題。緩和策：合成ループをSRAMに配置し、DMAの優先度を HIGH\_PRIORITY に設定する2。 |
| **PIO timingの破綻** | 高 | 低 | 複雑なステートマシンによる命令サイクルの枯渇。緩和策：1つのPIOブロックにつき最大32命令の制約内に収めるよう、処理を単純化する10。 |
| **LCD transfer bottleneck** | 中 | 必発 | SPIのハードウェア上限による転送遅延。緩和策：ダブルバッファリングによるチャンク転送でCPU処理との完全なオーバーラップを実現する8。 |
| **alignment / chunking問題** | 低 | 中 | タイル境界とチャンク境界の不一致による端数ピクセル計算の複雑化。緩和策：ダーティ矩形を強制的に16x16のタイル境界にスナップさせる設計とする。 |
| **Rust no\_std 制約** | 中 | 低 | 動的データ構造を多用した場合のメモリ確保の困難さ。緩和策：ヒープアロケータを避け、最大数を固定した静的配列とアリーナ確保で完結させる。 |
| **ライセンスの混入リスク** | 致命的 | 低 | Picoware等のGPL-3.0コードが混入しプロジェクト全体が汚染される危険性。緩和策：仕様やデータシートのみを参照し、Rustのコードベースは完全なクリーンルームから書き起こす5。 |
| **API肥大化** | 中 | 中 | KotoOSのホストコールが無限に増える問題。緩和策：特定のゲーム専用関数を拒否し、三層レイヤーの汎用APIのみに制限する。 |
| **テストと本番APIの混在** | 低 | 中 | テスト用コードがリリース版のSRAMを圧迫する問題。緩和策：Rustの \#\[cfg(test)\] や examples/ ディレクトリを活用し、本番環境から徹底的に排除する。 |

## **Recommendations**

アーキテクトとしての全体的な設計方針と開発における強力な推奨事項を以下に記載する。

### **最初に作るべきMVP**

最も重要かつ困難なマイルストーンは、Phase 1からPhase 3に相当する「PSRAM PIOドライバとLCD DMA転送の統合」である。VM連動やUIウィジェットの作成に着手する前に、ハードウェア実機上で1枚のタイルマップが正しく、かつ想定されたフレームレートでPartial Flush描画されることを最優先で実証すべきである。

### **やらないほうがいいこと**

最初からHTML/CSSレンダラーのような汎用GUIフレームワークや、動的なFlex/Gridレイアウト計算エンジンを目指すことは絶対に避けるべきである。RP2040のリソースと本構想の目的から大きく逸脱する。まずはゲーム、Shell、Memoの動作に必要な「絶対座標ベースの最小限のウィジェット」の完備に集中する。

### **設計上の境界線とKotoOS統合**

koto-psramクレートは、グラフィックスから完全に切り離された汎用バスドライバとして実装する。この厳格な分離により、同じPSRAMをオーディオバッファ10 やVMのバイトコードキャッシュとして安全に共有することが可能になる。また、KotoOSに統合する際は、PicoCalcのSTM32キーボードとのI2C通信において、通信速度を標準の10kHzから100kHz以上に設定し、ポーリングによる遅延が描画パイプラインをブロックしないよう非同期イベント駆動で入力状態を更新するアーキテクチャを徹底する13。

### **Rustらしい設計の徹底**

C/C++ベースのPicowareの実装手法（グローバル変数の多用やポインタによる直接メモリアクセス）をそのまま模倣してはならない。Rustの強力な型システム、Resultを用いた厳格なエラーハンドリング、ライフタイムと所有権を用いた安全なPsramRegionの引き回し設計を貫くこと。ただし、過度なジェネリクスや抽象化（例えば深いTrait境界のネスト）は、コンパイルサイズの肥大化やRP2040での実行時オーバーヘッドを招くため、実用性を重視した簡素な構造にとどめる。

## **最終的に判断したいこと**

提示された構想に対する最終的なアーキテクチャ判断は以下の通りである。

1. **この構想は技術的に妥当か**  
   完全に妥当である。RP2040のPIOとDMA、そしてチャンクベースの部分描画手法を組み合わせることで、SRAM容量の壁を突破し、PSRAMバックエンドによるグラフィックスエンジンは実現可能である。  
2. **まず作るべきMVPはどこまでか**  
   ハードウェアドライバ層を確立し、Tile Layerのチャンクベース描画とLCDへのPartial Flushが連動して動作する段階までである。  
3. **PSRAM 8MBのうちKotoGFXにどの程度割り当てるべきか**  
   Working Store（描画状態・ダーティリスト等）として約1MB、Graphics Assets（画像データ）として約2MBの、合計3MB程度の割り当てを推奨する。  
4. **RGB565ベースで始めるべきか、最初から4bpp indexedを入れるべきか**  
   MVP段階ではRGB565で始めるべきである。システムのパイプライン安定化を優先し、パレット展開に伴う論理的な複雑さは最適化フェーズまで先送りする。  
5. **Tile/Sprite/Text三層でShell/Memo/Launcher/Gameを十分表現できるか**  
   十分に表現可能である。テキストや背景にTile/Textレイヤーを用い、動的なカーソルやアイコンにSpriteレイヤーを組み合わせることで、対象とする全アプリケーションの要件を満たすことができる。  
6. **LVGL-inspiredという表現は妥当か** 妥当である。Retained Rendering、Dirty Invalidation、Partial FlushというLVGLの中核思想を正確に継承しており、アーキテクチャの方向性を説明する言葉として極めて適切である16。  
7. **Picoware-informedという表現は妥当か** 極めて妥当である。ハードウェアのピンアサインやQPI PSRAMの制御、STM32キーボードのI2C通信制約といったPicoCalc固有の実機知見を参考にしつつも、GPLコードを回避して独自実装することを端的に表している5。  
8. **Rust crate境界はどう切るべきか**  
   koto-psram, koto-display, koto-gfx, koto-game2d, koto-ui の5層への分割が、関心の分離と再利用性の観点から最適である。  
9. **VM hostcall APIはどの粒度にすべきか**  
   ピクセル単位やセル単位の細かな関数呼び出しではなく、矩形領域の塗りつぶしや配列を用いた一括更新（バッチ処理）の粒度とすべきである。これによりコンテキストスイッチの負荷を激減させる。  
10. **最も危険な設計ミスは何か** PSRAM読み出しとLCD転送を処理するDMAのシステムバス競合（Contention）を軽視し、クリティカルなレンダリングループのSRAM配置指定（\#\[link\_section \= ".data"\]等）を怠ることである。FlashキャッシュミスによりDMAが頻繁にストールすれば、激しいジッタが発生しプロジェクトは致命的なパフォーマンス低下に陥る2。これを防ぐメモリアライメントと優先度設計が最大の鍵となる。

#### **引用文献**

1. rp2040\_hal::dma \- Rust \- Docs.rs, [https://docs.rs/rp2040-hal/latest/rp2040\_hal/dma/index.html](https://docs.rs/rp2040-hal/latest/rp2040_hal/dma/index.html)  
2. How to control DMA channel priorities? \- Raspberry Pi Forums, [https://forums.raspberrypi.com/viewtopic.php?t=317794](https://forums.raspberrypi.com/viewtopic.php?t=317794)  
3. A header-only C library to allow access to SPI PSRAM via PIO on the RP2040 microcontroller. \- GitHub, [https://github.com/polpo/rp2040-psram](https://github.com/polpo/rp2040-psram)  
4. Tldr on flash and RAM in the Pico? \- Raspberry Pi Forums, [https://forums.raspberrypi.com/viewtopic.php?t=344055](https://forums.raspberrypi.com/viewtopic.php?t=344055)  
5. GitHub \- jblanked/Picoware: Open-source custom firmware for PicoCalc, Cardputer ADV, Video Game Module, and other ESP32/Raspberry Pi Pico devices, [https://github.com/jblanked/Picoware](https://github.com/jblanked/Picoware)  
6. CircuitPython working on the PicoCalc \- ClockworkPi Forum, [https://forum.clockworkpi.com/t/circuitpython-working-on-the-picocalc/20987](https://forum.clockworkpi.com/t/circuitpython-working-on-the-picocalc/20987)  
7. LennartHennigs/PicoCalc-Notes \- GitHub, [https://github.com/LennartHennigs/PicoCalc-Notes](https://github.com/LennartHennigs/PicoCalc-Notes)  
8. class DMA – access to the RP2040's DMA controller — MicroPython latest documentation, [https://docs.micropython.org/en/latest/library/rp2.DMA.html](https://docs.micropython.org/en/latest/library/rp2.DMA.html)  
9. Raspberry Pi PicoのDMAでシリアル通信する \- スマートライフを目指すエンジニア, [https://smtengkapi.com/raspberry-pi-pico-dma-serial](https://smtengkapi.com/raspberry-pi-pico-dma-serial)  
10. VGA driver using PIO and DMA on the RP2040 \- Hacker News, [https://news.ycombinator.com/item?id=36164564](https://news.ycombinator.com/item?id=36164564)  
11. Connect Raspberry Pi Pico to PSRAM with a library \#PiDay @Raspberry\_Pi \- Adafruit Blog, [https://blog.adafruit.com/2024/06/28/connect-raspberry-pi-pico-to-psram-with-a-library/](https://blog.adafruit.com/2024/06/28/connect-raspberry-pi-pico-to-psram-with-a-library/)  
12. Using DMA between Two State Machines, RP2040 Pico · micropython · Discussion \#15958, [https://github.com/orgs/micropython/discussions/15958](https://github.com/orgs/micropython/discussions/15958)  
13. I2C / Keyboard Speed \- PicoCalc \- ClockworkPi Forum, [https://forum.clockworkpi.com/t/i2c-keyboard-speed/21923](https://forum.clockworkpi.com/t/i2c-keyboard-speed/21923)  
14. Actual PicoCalc I2C usage and clocks? \- ClockworkPi Forum, [https://forum.clockworkpi.com/t/actual-picocalc-i2c-usage-and-clocks/19719](https://forum.clockworkpi.com/t/actual-picocalc-i2c-usage-and-clocks/19719)  
15. Actual PicoCalc I2C usage and clocks? \- Page 2 \- ClockworkPi Forum, [https://forum.clockworkpi.com/t/actual-picocalc-i2c-usage-and-clocks/19719?page=2](https://forum.clockworkpi.com/t/actual-picocalc-i2c-usage-and-clocks/19719?page=2)  
16. LVGL Graphics \- ESPHome \- Smart Home Made Simple, [https://esphome.io/components/lvgl/](https://esphome.io/components/lvgl/)