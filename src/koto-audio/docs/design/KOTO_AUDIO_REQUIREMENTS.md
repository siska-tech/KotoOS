# KotoAudio Requirements for PicoCalc

## 0. Scope

この文書は、KotoOS/PicoCalc/RP2040向けの KotoAudio 要求定義書である。KotoAudio は、P/ECE相当で必要十分な小型ゲーム機向け bounded audio runtime として、短い効果音、軽量なBGM、固定予算のmixer、抽象化されたaudio backend、PC側asset converterを提供する。

本設計は `docs/research/piece-analysis/02d_audio_realtime_model.md` の調査結果をもとにする。ただし、P/ECE互換APIは作らない。P/ECEのコード、ヘッダ、文書、素材はコピーせず、KotoOS独自の設計文書としてまとめる。P/ECE由来の名称をKotoAudioの公開API名としてそのまま採用しない。

前提:

- KotoOSはPicoCalc/RP2040向けに設計する。
- 通常アプリはaudio backend、PWM、PIO、DMA、timerへ直接アクセスしない。
- `koto-audio` がaudio mixerとbackendを所有する。
- アプリはclip、sequence、sourceの再生を非同期に要求する。
- KotoAudioは高機能DAWではなく、小型ゲーム機向けbounded audio runtimeである。
- 初期版ではstreamを後回しにする。
- 不明な実機性能値、最適sample rate、最適block sizeは未確定として扱い、実装時に計測で決める。

## 1. Design Principles

KotoAudioの基本方針は、音声を自由なI/Oではなく、時間制約つきの小さなsource処理系として扱うことである。

- Fixed budget: CPU時間、RAM、同時source数、queue深さ、decode数、backend buffer量に固定上限を置く。上限を超えた要求は失敗、drop、degrade、またはvoice stealingとして明示的に扱う。
- Bounded channels: 同時再生可能なsource数とbus数を固定する。アプリが実行時に無制限のchannelやcallbackを増やせる設計にはしない。
- Fixed block mixer: mixerは固定sample rate、固定block size、固定周期で動く。backendへ渡すbuffer契約もblock単位に揃える。
- App APIとrealtime backendの分離: アプリはlogical sourceを要求するだけで、PWM、PIO、DMA、timer、I2S、hardware bufferを所有しない。realtime処理はKotoAudio内部に閉じる。
- PC側asset conversion: WAV、sequence、MML風データ、tracker-like dataはPC側converterで検証・変換し、実機runtimeはcompactな実行準備済みassetを読む。
- Diagnostics first: underrun、late mix、dropped source、stolen voice、queue full、max mix time、backend stateを最初から観測可能にする。少なくともdebug buildではcounterをhostcallで読める。
- No user audio callback in ISR: 通常アプリのcallbackをISRやbackend refill直近の文脈で呼ばない。完了通知はevent queueまたはpoll型hostcallで配送する。

## 2. Non-goals

KotoAudio v0/v1では、以下を目標にしない。

- MP3/AAC/Opusなどの汎用圧縮音声decode。
- generic long streaming audio。
- stereo high quality mixing。
- runtime arbitrary resampling。
- plugin DSP、外部effect chain、任意filter graph。
- app-owned audio callback。
- 通常アプリによるraw PWM/PIO/DMA/timer/backend access。
- P/ECE互換API、P/ECE asset形式互換、P/ECE muslib互換。
- DAW的な編集、録音、波形表示、リアルタイム音声合成環境。

Streamは将来予約するが、初期版では長尺音声やSDカードからの継続streamingを要件に入れない。

## 3. Audio Source Model

KotoAudioが公開する基本単位はsourceである。sourceはclip、sequence、将来予約streamのいずれかを参照し、mixer内のslotへ非同期に投入される。

### Clip

Clipは短いmemory-resident soundである。主用途はSFX、UI音、短いジングルである。runtimeでのdecodeやresamplingを最小化するため、PC側converterがKotoAudio向けの固定形式へ変換しておく。

要求:

- 短時間再生を前提にする。
- 変換済みPCMまたは軽量なruntime decode形式を許容するかは未確定であり、v0ではPCM優先とする。
- loop点を持てるが、v0では単純loopまたは全体loopに限定してよい。
- 再生完了時にcompletion eventを発行できる。

### Sequence

SequenceはBGM向けの軽量な時系列音楽データである。note、instrument、tempo、loop、volume eventなどをPC側でcompactな中間表現に変換し、runtimeは固定上限内で解釈する。

要求:

- v0ではBGM busまたはplaceholderとして予約し、完全なsequence再生はv1で扱う。
- v1では小さなinstrument/wavetable、drum、loop、priorityと共存できる。
- sequence内部voice数、instrument数、event rateには固定上限を置く。
- BGMとSFXは同じmixer budget内で共存する。

### Stream

Streamは将来予約のsource種別である。SDカード、host、またはproducer-fed bufferからの長尺音声を想定しうるが、v0/v1の中心要件にはしない。

要求:

- 初期ABIでは予約IDまたは未対応エラーに留めてよい。
- stream追加時も通常アプリへraw backend bufferを渡さない。
- 長尺streamingを追加する場合はstorage、scheduler、power、underrun policyを別途定義する。

### Source Metadata

すべてのsource requestは、以下のmetadataを持つ。

| Field | Requirement |
|---|---|
| source id | runtimeが返す不透明ID。stop、volume変更、state query、completion eventの照合に使う。 |
| source priority | 過負荷時、queue満杯時、voice stealing時の優先度。system soundは通常app soundより高くできる。 |
| volume | source-local volume。app volume、bus volume、master volumeと合成して適用する。 |
| loop | one-shot、有限loop、無限loopを表現する。v0ではclip単純loopを優先する。 |
| state | queued、playing、paused、stopping、completed、dropped、errorなどを持つ。 |
| completion event | 再生完了、drop、steal、stop、errorをeventとして観測できる。ISR内user callbackではない。 |

## 4. Mixer Requirements

MixerはKotoAudioの実時間中心であり、アプリから独立して固定周期で動く。

要求:

- Fixed sample rate: v0では単一sample rateに固定する。候補値は16kHzまたは22.05kHzだが、PicoCalc/RP2040 backend計測後に決める。
- Mono first: v0/v1はmonoを第一対象にする。stereo high quality mixingは非目標とする。
- Fixed block size: mixer block sizeはbuildまたはbackend policyで固定する。候補値は未確定であり、latency、CPU負荷、DMA refill marginで決める。
- Bounded source count: active source数、queued source数、sequence voice数に固定上限を置く。
- Integer mixing preferred: 実機runtimeでは整数演算を優先する。浮動小数点DSPを前提にしない。
- Master/app/source volume: master volumeはsystem所有、app volumeはアプリ単位、source volumeはsource単位で適用する。v1ではbus volumeも追加する。
- Clipping/saturation: mix結果はbackend formatへ変換する前にsaturationまたは定義済みclipping処理を行う。
- Silence on underrun: sourceがない、mixerが間に合わない、backend bufferが不足した場合は安全な無音またはneutral sampleを出す。
- Low priority source drop/degrade: 予算超過時は低優先sourceをdrop、degrade、またはvoice stealする。無制限にmix時間を延ばさない。
- No app blocking: play requestはmixer完了を待ってVMをblockしない。queue満杯などは即時statusまたはeventで返す。

Mixerは通常アプリの所有物ではない。アプリはsource要求を出すだけで、mixer block、backend buffer、ISR周期には触れない。

## 5. Backend Boundary

Backendは、mixerが生成したblockを物理出力へ渡す内部境界である。通常アプリからは見えない。

Backend abstraction requirements:

- PWM/PIO/I2S/DMA backend abstractionを持つ。実機ではPWM、PIO、DMA、timer、I2S相当のどれを使ってもよいが、mixer-facing APIは共通にする。
- Backend start/stopをKotoAudio内部で管理する。通常アプリはbackendを起動・停止しない。
- Buffer submitをblock単位で受ける。backendはdouble buffer、ring buffer、DMA queueなどの内部実装を隠す。
- Underrun detectionを持つ。backend側でsubmitが間に合わない場合はcounterを増やし、可能なら無音を出す。
- Power suspend/resume hooksを持つ。power managerからのquiesce、suspend、resumeへ応答できる。
- Backend stateはdebug buildまたはsystem app diagnosticsだけが観測できる。
- Simulator/mock backendとPicoCalc backendは同じmixer-facing APIを共有する。

Backend hidden from normal app:

- 通常アプリはPWM level、PIO program、DMA channel、timer compare、I2S frame、backend buffer pointerを取得できない。
- 通常アプリはbackend policyを変更できない。
- 通常アプリはbusy-waitでbackend空きを待たない。

## 6. Asset Converter Requirements

KotoAudioはPC側asset conversionを前提にする。実機runtimeは、検証済みでcompactなruntime-ready assetを読む。

Clip converter requirements:

- WAV to KotoAudio clipをサポートする。
- 入力WAVのsample rate、duration、channels、bit depth、loop point、最大振幅を検証する。
- v0 runtime sample rateと一致しない場合はPC側で変換するか、明示的に拒否する。runtime arbitrary resamplingはしない。
- mono runtime向けにstereo入力を拒否またはPC側downmixする。
- memory budgetを検証し、RAM/flash配置想定をreportする。
- loop pointがblock境界、sample境界、asset範囲に収まるか検証する。
- compact runtime-ready assetを出力する。

Sequence converter requirements:

- sequence、MML風データ、tracker-like dataからKotoAudio music assetを生成する。
- v1向けにnote event、tempo、loop、instrument、wavetable、drum、volume eventをcompact表現へ変換する。
- runtime sequence voice数、instrument数、event密度、loop構造、asset sizeを検証する。
- P/ECEのMML文法やmuslib形式への互換を目標にしない。

Diagnostics requirements:

- estimated CPU budgetをreportする。例: active clip数、sequence voice数、decode有無、expected mix load。
- estimated memory budgetをreportする。例: asset size、runtime metadata、queue slot、instrument/wavetable size。
- 変換時に警告とエラーを分ける。runtimeで危険な条件は可能な限りconverterで止める。
- 出力assetにはformat version、sample rate、channel count、duration、loop metadata、budget hintsを含める。

## 7. Hostcall Boundary

Hostcallは、通常アプリ、システムアプリ、debug-onlyで明確に分ける。

### Normal App Hostcalls

通常アプリはlogical source操作だけを要求できる。

| Hostcall | Requirement |
|---|---|
| `play_clip` | 変換済みclip assetをsourceとしてenqueueする。source idまたは失敗statusを返す。 |
| `play_sequence` | 変換済みsequence assetをBGMまたはmusic sourceとしてenqueueする。v0では未対応またはplaceholderでもよい。 |
| `stop` | source id、app全体、または許可されたscopeの再生を停止する。 |
| `set_source_volume` | 指定sourceのvolumeを変更する。 |
| `set_app_volume` | app単位のvolumeを変更する。master volumeには触れない。 |
| `poll_audio_event` | completion、dropped、stolen、errorなどのeventをpollする。 |
| `query_audio_counters` | 通常アプリ向けに許可されたcounterを読む。詳細backend stateは含めない。 |

### System App Hostcalls

システムアプリはユーザー設定、foreground policy、power policyのために広い制御を持てる。

| Hostcall | Requirement |
|---|---|
| `set_master_volume` | OS全体のmaster volumeを設定する。 |
| `mute` | user/system policyに基づき全体muteを設定する。 |
| `audio_focus` | foreground/background、system sound、launcher/menuのfocusを管理する。 |
| `backend_policy` | backend選択、latency/power policyなどを設定する。通常アプリには公開しない。 |
| `diagnostics_reset` | audio countersをresetする。debug buildではより詳細なresetも許可できる。 |

### Debug-only Hostcalls

Debug-only hostcallはrelease通常アプリABIではない。

| Hostcall | Requirement |
|---|---|
| `dump_mixer_load` | mix time、max mix time、active source数、voice数、queue depthをdumpする。 |
| `dump_backend_state` | backend running state、buffer fill、DMA/PIO/PWM/I2S状態の抽象情報をdumpする。 |
| `dump_underrun_counters` | mixer/backend underrun、late submit、silence fill回数をdumpする。 |
| `force_underrun_test` | 診断用にunderrun経路を強制し、silence/counter/event動作を検証する。 |

## 8. Power Integration

KotoAudioはpower managerと明示的に連携する。音声出力中のsuspend/resumeは、ノイズ、underrun、backend状態破損を避けるため、service境界で扱う。

要求:

- Suspend request: power managerはKotoAudioへsuspend準備を要求できる。
- Quiesce: KotoAudioは新規source受付停止、queue drain、fade out、即時停止のいずれかをpolicyに従って選ぶ。policyはsystem側が所有する。
- Silent prefill: backend停止前またはresume直後に無音blockをprefillし、出力の不定値を避ける。
- Resume: backend再初期化、buffer prefill、mixer state再開、またはsource停止済み扱いを明確にする。
- Pop-noise avoidance: amplifier、PWM duty、I2S clock、DMA start/stopの順序はbackend内部で制御し、ポップノイズを減らす。
- Active audio keepalive policy: active audioがある場合、system policyによりsuspendを延期、許可、fadeして停止、またはmuteして継続できる。
- Counters: suspend中drop、resume underrun、forced silence、backend restart回数をdebug counterとして観測できる。

未確定:

- PicoCalc実機でのアンプ制御GPIO、PWM/PIO/I2S構成、resume時の最小silent prefill量は未確認である。
- active audioがsuspend vetoできるか、単なるkeepalive hintに留めるかはKotoOS power policyで決める。

## 9. v0/v1 Roadmap

### v0

v0はSFX中心の最小bounded audio runtimeである。

- SFX clip。
- Fixed sample rate mono。
- 4 SFX sources。
- 1 BGM placeholderまたはreserved bus。
- Source queue。
- Stop/loop。
- Volume。
- Counters。
- WAV converter。

v0の追加要件:

- 通常アプリは`play_clip`でVMをblockせずにSFXを鳴らせる。
- queue満杯、source不足、asset不正はstatusまたはeventとして扱う。
- backendはmock/simulatorとPicoCalc実機で同じ上位APIを持つ。
- underrun、dropped source、queue full、active source countを観測できる。

### v1

v1はBGMとSFXの共存を実用化する。

- BGM sequence。
- Small instruments/wavetable。
- Priority/voice stealing。
- Fade。
- App/bus volume。
- Backend abstraction polish。

v1の追加要件:

- BGM sequenceとSFX clipが同一fixed budget内で共存する。
- BGM bus、SFX bus、app volume、source volume、master volumeの関係を定義する。
- voice stealingはpriority、source type、system focusに従う。
- fadeはsource fade、bus fade、BGM stop fadeを最小範囲で持つ。
- asset converterはsequence budgetもreportする。

### Later

v1以降に検討するが、初期要求には含めない。

- Stream source。
- 長尺BGM streaming。
- より高品質なsample rate conversion。
- stereo output。
- effect DSP。
- 外部codecやhost-fed audio。

## 10. Acceptance Criteria

KotoAudio v0/v1は、少なくとも以下を満たす。

- KotoBlocks can play SFX without blocking VM.
- BGM and SFX can coexist within fixed budget.
- Underrun and dropped source counters are observable in debug.
- Normal app cannot access raw audio hardware.
- Simulator/mock backend and PicoCalc backend share the same API.

詳細な受け入れ条件:

- 通常アプリからPWM、PIO、DMA、timer、backend bufferへ到達する公開hostcallが存在しない。
- `play_clip`は非同期に戻り、再生完了はcompletion eventで観測できる。
- source数上限を超えた場合の動作が、失敗、drop、degrade、voice stealingのいずれかとして定義されている。
- mixerがsource不足またはbackend underrun時に無音を出し、counterを増やす。
- master volumeはsystem appだけが変更でき、通常アプリはapp/source volumeだけを変更できる。
- asset converterがsample rate、duration、channels、loop point、memory budgetを検証する。
- mock backendでsource lifecycle、event、counter、drop policyをテストできる。
- PicoCalc backendは同じmixer-facing APIで差し替えられる。

## 11. Unknowns and Measurement Items

以下は設計時点で不明として扱い、実装前または実装中に計測で決める。

- v0 fixed sample rateを16kHz、22.05kHz、または別値にするか。
- mixer fixed block sizeとbackend buffer depth。
- RP2040上で4 SFX sources + 1 BGM reserved busを動かす場合のworst-case mix time。
- PicoCalc実機のaudio出力方式、アンプ制御、ポップノイズ特性。
- PCMのみでv0を成立させるか、軽量ADPCM相当を早期に入れるか。
- sequence v1の最大voice数、instrument数、event密度。
- active audio keepaliveがsuspendを拒否できるか、system policyが常に優先するか。

## 12. Design Conclusion

KotoAudioは、P/ECEから得られる「素材を前処理し、実時間処理を固定予算へ閉じ込め、アプリには非同期で小さな命令だけを見せる」という設計思想を、KotoOS/PicoCalc向けに再定義する。

重要なのは互換性ではなく境界である。アプリはclipやsequenceを要求する。KotoAudioはsource、mixer、backend、diagnosticsを所有する。backendはPWM、PIO、DMA、timer、I2Sの詳細を隠す。asset converterは実機で危険な自由度をPC側で潰す。debug diagnosticsは最初から用意し、音切れやdropを見える失敗として扱う。

この範囲に留めることで、KotoAudio v0/v1は高機能DAWではなく、小型ゲーム機に必要十分なbounded audio runtimeとして成立する。
