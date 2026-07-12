# KotoAudio v0/v1 Architecture

## 0. Scope

この文書は、`KOTO_AUDIO_REQUIREMENTS.md` と
`KOTO_AUDIO_CODEC_POLICY.md` をもとに、KotoAudio v0/v1 を実装可能な
crate/module 構成、データフロー、型責務、backend 境界、hostcall 接続へ
落とし込むためのアーキテクチャ設計書である。

`KOTO_AUDIO_CODEC_EXPERIMENT_PLAN.md` は、codec 採否と未確定値を測定で決める
ための参考文書として扱う。

KotoAudio は、小型ゲーム機向けの bounded audio runtime である。v0 は
PCM16 mono の短い SFX clip を中心にし、stream と本格的な sequence/BGM は
後回しにする。SLDPCM4 は v0 の中心 codec ではなく、KotoAudio 独自の codec
拡張ポイント、かつ実験候補として扱う。通常アプリは audio backend、PWM、PIO、
DMA、timer へ直接アクセスしない。

この文書は実装コードを書かない。公開 API 名として既存 P/ECE 互換名は使わず、
KotoAudio 独自設計として記述する。

## 1. Architecture Overview

KotoAudio は、アプリからの非同期再生要求を、固定予算の source、decoder、
mixer、backend へ変換する runtime service である。アプリに見える境界は
logical source 操作と event/counter のみであり、realtime backend と hardware
detail は KotoAudio 内部に閉じる。

層構成:

- app hostcall layer:
  normal app、system app、debug-only の hostcall を受け、権限に応じて
  `AudioService` へ委譲する。normal app には clip/sequence/source/event/counter
  の logical 操作だけを公開する。
- audio service:
  KotoAudio の所有者であり、source admission、policy、mixer tick、backend
  submit、event/counter 更新、power integration を統合する。
- source manager:
  source id、queue、active slot、priority、loop、stop、completion を管理する。
  bounded な固定数 slot を持ち、上限超過を drop、reject、steal として明示する。
- decoder layer:
  clip asset の codec を PCM16 mono sample stream へ展開する。v0 required は
  PCM16、PCM8 は optional、SLDPCM4 は experimental extension、ADPCM4 は
  future/comparison 候補とする。
- mixer:
  固定 sample rate、固定 block size、mono、整数演算を基本に、active source から
  `MixerBlock` を生成する。signed 32bit accumulator を使い、最終的に backend
  向け format へ saturation/clipping して渡す。
- backend abstraction:
  mock、simulator、PicoCalc 実機 backend が共有する mixer-facing API。PWM、PIO、
  DMA、timer、I2S、buffer queue の詳細は backend 内部に隠す。
- diagnostics:
  active/queued/dropped/stolen/underrun/late mix/max mix time/backend restart などを
  最初から持つ。debug build では詳細 state を読めるが、normal app には許可された
  counter だけを返す。
- asset converter boundary:
  PC 側 converter が WAV、loop、sample rate、channel、duration、codec、memory
  budget を検証し、runtime-ready asset を出力する。runtime は arbitrary resampling
  や危険な asset repair を行わない。

## 2. Data Flow

KotoAudio の通常再生フローは、hostcall から mixer/backend まで非同期に進む。
アプリは再生完了を callback ではなく event queue で観測する。

`play_clip` hostcall:

- normal app が clip asset handle、volume、priority、loop request を渡す。
- hostcall layer が app 権限と引数範囲を検査する。
- `AudioService::play_clip` 相当の責務へ委譲する。
- 成功時は不透明な `SourceId` を返す。失敗時は immediate status を返し、必要に
  応じて drop/error event と counter を更新する。

asset lookup:

- `AudioService` は asset registry または app-owned asset table から `ClipAsset`
  を解決する。
- sample rate、channel count、codec id、duration、loop metadata、placement hint を
  runtime header から確認する。
- 不正 asset は `malformed_asset_count` を増やし、source を admitted しない。

source allocation:

- `SourceManager` が queue slot または active slot を確保する。
- 空きがない場合は policy に従い、低 priority source を steal するか、新規 request
  を drop/reject する。
- v0 では SFX source count 4 を基本案にし、BGM reserved bus は placeholder とする。

source queue insertion:

- accepted source は `queued` 状態で source queue に入る。
- queue が満杯なら `queue_full_count` と `dropped_source_count` を更新する。
- queue 挿入は app を mixer 完了まで block しない。

mixer block generation:

- mixer tick は固定 block 単位で active source を走査する。
- queued source は slot に昇格され、decoder state が初期化される。
- decoder は codec に関係なく PCM16 mono sample を mixer へ返す。
- mixer は source/app/master volume を適用し、signed 32bit accumulator に加算する。
- source 終端、loop、stop、steal、error は source state と event 生成候補へ反映する。

backend buffer submit:

- `MixerBlock` は backend abstraction の `submit_block` 相当へ渡される。
- backend は double buffer、ring buffer、DMA queue などの実装詳細を隠す。
- submit できない、または backend が refill に間に合わない場合は無音を出し、
  underrun を report する。

completion/drop/underrun event generation:

- one-shot 完了、有限 loop 完了、明示 stop、drop、steal、asset error、backend
  underrun は `AudioEvent` として event queue に入る。
- event queue が満杯の場合、counter を増やし、debug diagnostics で観測可能にする。
- user callback は ISR や backend refill 文脈では呼ばない。

`poll_audio_event`:

- normal app は自分の app scope に属する event だけを poll する。
- system app は policy に応じて system-wide event を読める。
- event は `SourceId` と reason を含むが、codec state や backend pointer は含めない。

`query_audio_counters`:

- normal app は許可された counter の snapshot を読む。
- system/debug は backend state、mixer load、underrun detail を追加で読める。
- counter は runtime の異常を隠さず、音切れ、drop、late mix を見える失敗にする。

## 3. Proposed Module Layout

crate 配置は `crates/koto-audio` を第一候補にする。workspace 方針によっては
`koto-audio/src` 配下でも同じ責務分割にできる。

v0 required modules:

- `lib.rs`: public crate boundary、feature flags、主要型の再 export。
- `service.rs`: `AudioService`、service lifecycle、play/stop/volume/poll/counter。
- `source.rs`: `SourceManager`、`SourceId`、`AudioSource`、`SourceSlot`、queue/slot 管理。
- `clip.rs`: `ClipAsset` metadata、clip playback request、loop metadata。
- `decoder.rs`: decoder trait/dispatcher、PCM16 output contract。
- `codec/pcm16.rs`: v0 required PCM16 mono decoder。
- `mixer.rs`: fixed block mixer、volume、saturation、completion detection。
- `backend.rs`: mixer-facing backend abstraction、state、submit contract。
- `event.rs`: `AudioEvent`、event queue、event filtering。
- `counters.rs`: `AudioCounters`、debug/system counter snapshot。
- `policy.rs`: `AudioPolicy`、priority、drop/steal、limits、power policy hooks。
- `asset.rs`: runtime-ready asset header validation、asset placement hints。
- `hostcall.rs`: normal/system/debug hostcall adapter。
- `backend/mock.rs`: deterministic tests 用 mock backend。

v0 optional or early integration modules:

- `codec/pcm8.rs`: optional PCM8 decoder。PCM16 mixer contract へ展開する。
- `backend/sim.rs`: simulator audio output、visualized buffer/counter inspection。
- `backend/picocalc.rs`: PicoCalc 実機 backend 候補。hardware detail は内部に隠す。

v1以降または experimental modules:

- `sequence.rs`: `SequenceAsset`、BGM/sequence source、tempo/event interpreter。
- `codec/sldpcm4.rs`: experimental SLDPCM4 decoder extension。
- `codec/adpcm4.rs`: future/comparison ADPCM4 decoder 候補。
- `stream.rs`: later reserved。v0/v1 の中心要件にはしない。
- `bus.rs`: v1 の SFX/BGM bus、bus volume、fade policy。
- `focus.rs`: system app 向け `AudioFocus` policy を独立させる場合の候補。

## 4. Core Responsibility Model

`AudioService`:

- KotoAudio runtime の単一の調停者。
- hostcall adapter からの要求を受け、asset validation、source admission、mixer tick、
  backend submit、events、counters、power hooks を接続する。
- normal app に backend/hardware detail を見せない。

`SourceManager`:

- source queue、active source slot、priority、state transition を管理する。
- bounded limits を超えた要求を reject/drop/steal として処理する。
- source lifecycle event の生成元になる。

`SourceId`:

- app が stop、volume、event 照合に使う不透明 ID。
- slot index そのものを露出しない。generation を含め、古い ID の誤操作を防ぐ設計が望ましい。

`AudioSource`:

- clip、sequence、将来 stream などの logical source request。
- volume、priority、loop、app owner、state、asset reference を持つ。
- codec predictor や backend buffer pointer は持たない。

`SourceSlot`:

- mixer が実際に処理する bounded slot。
- source metadata、decoder instance/state、sample cursor、loop state、fade/stop 状態を持つ。
- slot は `SourceId` と generation で照合される。

`ClipAsset`:

- runtime-ready clip metadata と payload reference。
- codec id、sample rate、mono、sample count、duration、loop metadata、placement hint、
  budget hint を持つ。
- v0 は PCM16 mono at mixer rate を required とする。

`SequenceAsset`:

- v1 の BGM/sequence 用 runtime-ready metadata。
- note/event/instrument/wavetable/loop/tempo を固定上限内に収める。
- v0 では placeholder または unsupported として扱う。

`Decoder`:

- codec-specific payload を PCM16 mono sample stream に変換する。
- codec state を source slot 内に閉じ、normal app API へ漏らさない。
- end-of-clip、loop、malformed payload を source state へ返す。

`Mixer`:

- fixed sample rate/block size で active source を mix する。
- signed 32bit accumulator、整数 volume、saturation/clipping を担当する。
- max mix time と late mix を diagnostics へ report する。

`MixerBlock`:

- backend に渡す固定長 block。
- v0 は mono PCM16 相当の内部表現を基本にし、backend format への変換は backend または
  backend adapter 側に閉じる。
- block size は build/policy で固定する。

`AudioBackend`:

- start/stop、submit_block、query_state、underrun reporting、suspend/resume hooks を持つ
  mixer-facing 境界。
- mock/sim/PicoCalc backend は同じ API を実装する。
- PWM、PIO、DMA、timer は backend 内部実装に閉じる。

`AudioEvent`:

- completion、stopped、dropped、stolen、error、underrun などを表す poll 型 event。
- callback ではなく bounded event queue で配送される。
- source id、app owner、reason、軽量な detail を持つ。

`AudioCounters`:

- runtime の失敗と負荷を観測する counter snapshot。
- active/queued/dropped/stolen/underrun/late mix/max mix time/backend restart/queue full/
  malformed asset を含む。

`AudioPolicy`:

- limits、priority、drop/steal、volume bounds、power quiesce、backend policy をまとめる。
- normal app は policy を直接変更しない。system app または build config が所有する。

`AudioFocus`:

- foreground/background、system sound、mute、focus loss 時の source admission/volume/stop を
  決める system-level concept。
- v0 では最小 policy、v1 以降で BGM/SFX/fade と統合する。

## 5. v0 Static Limits

v0 は固定上限を先に置き、実機測定で値を調整する。未確定値は未確定として扱い、
測定項目に残す。

初期案:

| Item | v0 proposal | Status |
|---|---|---|
| sample rate | 16kHz または 22.05kHz | 未確定。PicoCalc/RP2040 backend 測定で決定 |
| channels | mono only | v0 required |
| block size | 128、256、または backend deadline に合う固定値 | 未確定。latency、DMA margin、mix time で決定 |
| SFX source count | 4 | v0 基本案 |
| BGM reserved bus | 1 placeholder | v0 では reserve のみ |
| source queue depth | 8 程度の固定 queue | 未確定。KotoBlocks 連打 SFX を測定 |
| event queue depth | 16 程度の固定 queue | 未確定。completion/drop burst を測定 |
| max clip duration | 1から3秒程度、または memory budget 制約 | 未確定。asset set と RAM/flash 配置で決定 |
| memory budget | PCM16 で成立する SFX set を基準 | 未確定。SRAM/PSRAM/flash/SD 方針で決定 |
| asset placement | SRAM は hot/small、PSRAM は larger resident、flash 相当は read-only、SD は v0 stream 対象外 | 未確定。実機 I/O と latency で決定 |
| decode budget | PCM16 4 sources + BGM placeholder で block deadline の 50% 以下を目標 | 未確定。実測で決定 |

asset placement policy:

- SRAM: 最も確実な v0 hot SFX 置き場。短い UI/SFX に向く。
- PSRAM: 容量に余裕があるが latency と bus contention を測定する。
- flash 相当: read-only resident asset 候補。random access cost を backend/mixer deadline と分離する。
- SD: v0 の generic stream には使わない。clip preload または converter/install 時配置の候補に留める。

## 6. Decoder and Codec Architecture

decoder layer の契約は「codec payload から PCM16 mono sample を mixer へ出す」ことである。
codec 選択、decode table、predictor、step index、payload nibble offset は normal app API へ
漏らさない。

v0 required:

- PCM16 mono decoder。
- asset sample rate は mixer sample rate と一致する。
- decoder state は sample cursor のみでよい。
- PCM16 は converter、tests、golden output の基準形式でもある。

v0 optional:

- PCM8 decoder。
- mixer へ渡す前に PCM16 mono へ展開する。
- UI/noise 系など用途限定 profile として扱い、品質 gate と converter report を前提にする。

v0 experimental extension:

- SLDPCM4 decoder。
- KotoAudio 独自 codec として build flag または asset-pipeline flag の背後に置く。
- fixed logarithmic delta table、step shift、previous sample、nibble parity、loop predictor seed などの
  codec state を source slot 内に閉じる。
- 測定で有用性が確認されるまで v0 compatibility の前提にしない。

future/comparison:

- ADPCM4 decoder。
- predictor、step index、block restart、loop checkpoint の仕様が増えるため v0 の中心にはしない。
- v1 stable codec 候補として SLDPCM4 と比較する。

loop state:

- PCM16/PCM8 は decoded sample cursor だけで loop できる。
- SLDPCM4/ADPCM4 は loop start へ戻る時に decoder state を復元する必要がある。
- loop point は decoded PCM sample index で表し、`loop_start` inclusive、`loop_end` exclusive とする。
- runtime は loop のたびに sample 0 から scan しない。converter が checkpoint または explicit loop state を
  asset metadata に持たせる。
- v0 は whole-clip loop と simple forward loop を優先し、ping-pong/crossfade/pitch shift loop は扱わない。

## 7. Mixer Architecture

mixer は fixed block runtime の中心である。

方針:

- fixed sample rate、fixed block size。
- v0/v1 は mono。
- integer mixing preferred。浮動小数点 DSP を前提にしない。
- decoder output は signed PCM16 mono。
- source volume、app volume、master volume を適用する。v1 では bus volume を追加できる。
- accumulator は signed 32bit。
- final output は backend contract に渡す前に saturation/clipping する。
- source 不足、late mix、backend underrun 時は silence を出す。
- budget 超過時は low priority source を drop/steal する。
- max mix time、late mix count、active source count を counter に反映する。
- app は mixer 完了を待って block しない。

block generation の責務:

- queued source を active slot へ昇格する。
- active slot ごとに decoder を進める。
- end-of-source と loop を処理する。
- stop/stolen/dropped/error の event を予約する。
- block deadline を超えた場合は `late_mix_count` を増やし、必要なら無音または部分 mix の扱いを policy で決める。

## 8. Backend Architecture

`AudioBackend` 相当の責務:

- `start`: hardware または simulated output を開始し、必要な silent prefill を行う。
- `stop`: 出力を安全に止める。pop noise 回避の詳細は backend 内部。
- `submit_block`: mixer が生成した固定 block を受け取る。
- `query_state`: running、buffer fill、last error、underrun 状態などの抽象 state を返す。
- `report_underrun`: backend 側 underrun を service/counters/events へ伝える。
- `suspend`: power quiesce 後、backend を停止または silent state へ移す。
- `resume`: backend 再初期化、silent prefill、running state 復帰を行う。

backend 種別:

- mock backend:
  deterministic tests 用。submitted block を記録し、manual underrun、queue full、state transition を
  再現できる。実音声出力は不要。
- simulator backend:
  desktop/simulator 用。実際の audio device または simulated buffer に接続できるが、mixer-facing
  API は mock/PicoCalc と同じにする。buffer fill や waveform inspection を debug に使える。
- PicoCalc backend:
  実機候補。PWM、PIO、DMA、timer、I2S 相当のどれを使うかは未確定。選ばれた方式の register、
  DMA channel、PIO program、timer interrupt は backend 内部に隠す。

通常アプリは backend policy を変更できず、PWM level、PIO program、DMA channel、timer compare、
backend buffer pointer を取得できない。

## 9. Hostcall Adapter

hostcall adapter は権限別の薄い境界であり、実処理は `AudioService` へ委譲する。

normal app hostcalls:

| Hostcall | Delegated responsibility |
|---|---|
| `play_clip` | asset lookup、source admission、queue insertion、`SourceId` 発行 |
| `play_sequence` | v0 は unsupported/placeholder、v1 は sequence source admission |
| `stop` | source/app scope stop、event 生成、slot 解放または fade request |
| `set_source_volume` | source-local volume 更新 |
| `set_app_volume` | app volume 更新。master volume は変更不可 |
| `poll_audio_event` | app scope の event queue poll |
| `query_audio_counters` | normal app に許可された counter snapshot |

system app hostcalls:

| Hostcall | Delegated responsibility |
|---|---|
| `set_master_volume` | master volume 更新、mute/focus policy と合成 |
| `mute` | system mute policy 更新、必要なら mixer 出力を silence |
| `audio_focus` | foreground/background、system sound、focus loss policy |
| `backend_policy` | backend selection、latency/power policy。normal app には非公開 |

debug-only hostcalls:

| Hostcall | Delegated responsibility |
|---|---|
| `dump_mixer_load` | active/queued count、mix time、max mix time、late mix |
| `dump_backend_state` | abstract backend state、buffer fill、running/suspended |
| `dump_underrun_counters` | mixer/backend underrun、late submit、silence fill |
| `force_underrun_test` | mock/sim/debug backend 経由で underrun path を強制 |

## 10. Power Integration

KotoAudio は power manager からの suspend/resume を service 境界で受ける。

quiesce sequence:

- 新規 source admission を停止する。
- 既存 queue を drain、drop、または stop する。選択は `AudioPolicy` が決める。
- active source は fade または immediate silence にする。v0 は immediate silence を許容し、
  fade は v1 polish として扱える。
- backend 停止前に silent block を prefill する。
- backend を stop/suspend する。
- suspend 中 drop、forced silence、backend stop を counter に反映する。

resume sequence:

- backend を再初期化する。
- silent prefill を行い、不定値や pop noise を避ける。
- source state policy に従い、既存 source を stopped 扱いにするか、paused から再開するかを決める。
- v0 では suspend 時に active source を stopped/completed ではなく stopped/drop reason として扱う案が単純。
- backend restart count、resume underrun、forced silence を counter に反映する。

未確定:

- active audio が suspend を veto できるか、system policy が常に優先するか。
- PicoCalc 実機の amplifier/GPIO、PWM duty、DMA/PIO/I2S start/stop 順序。
- resume 時の silent prefill block 数。

## 11. Diagnostics and Counters

KotoAudio は diagnostics を後付けにしない。v0 から以下の counter を持つ。

required counters:

- `active_source_count`: 現在 active な source 数。
- `queued_source_count`: queue 内 source 数。
- `dropped_source_count`: admission 失敗や policy drop の累計。
- `stolen_source_count`: priority policy により置換された source の累計。
- `underrun_count`: mixer/backend underrun の累計。必要なら subtype を debug に持つ。
- `late_mix_count`: block deadline に間に合わなかった回数。
- `max_mix_time`: 観測された最大 mix time。単位は実装時に ticks/usec から選ぶ。
- `backend_restart_count`: backend start/restart/resume 再初期化回数。
- `queue_full_count`: source/event queue full のうち source queue full 回数。
- `malformed_asset_count`: runtime validation で不正 asset を検出した回数。

additional debug candidates:

- event queue full count。
- silence fill count。
- decoder error count by codec。
- per-backend underrun count。
- max submit latency。

normal app には必要最小の snapshot を返し、backend raw state は system/debug に限定する。

## 12. Testing Strategy

v0 の test は mock backend を中心に source lifecycle と bounded failure を検証する。

test targets:

- mock backend で source lifecycle を検証する。
- PCM16 clip 再生で submitted block が非無音になることを確認する。
- queue full 時に status/event/counter が一貫することを確認する。
- source completion event が `poll_audio_event` で取得できることを確認する。
- stop と loop の state transition を検証する。
- forced underrun で `underrun_count` と event/report が増えることを確認する。
- mock backend と simulator/PicoCalc backend が同じ mixer-facing API で差し替え可能であることを確認する。
- normal app が raw backend、PWM、PIO、DMA、timer に触れないことを API surface test で確認する。
- KotoBlocks SFX acceptance として、VM を block せず `play_clip` 連打、completion poll、queue full を扱えることを確認する。

codec tests:

- PCM16 は golden path として必須。
- PCM8 は optional build で PCM16 変換と品質 gate report を確認する。
- SLDPCM4 は experimental build で loop state、malformed payload、fallback 前提を確認する。

## 13. Implementation Roadmap

Phase 1: skeleton + mock backend + PCM16 source lifecycle

- crate/module skeleton。
- `AudioService`、`SourceManager`、`SourceId`、mock backend。
- PCM16 clip asset validation と queued/playing/completed lifecycle。

Phase 2: mixer + counters + events

- fixed block mixer。
- source/app/master volume の最小実装。
- event queue と required counters。
- queue full、drop、completion、underrun path。

Phase 3: hostcall adapter integration

- normal app hostcalls。
- system app volume/focus/policy の最小 hook。
- debug-only diagnostics hostcalls。

Phase 4: simulator backend

- simulator backend を同じ `AudioBackend` API に接続。
- buffer/counter inspection。
- KotoBlocks SFX acceptance の simulator 検証。

Phase 5: PicoCalc backend

- PicoCalc 実機出力方式の選定。
- PWM/PIO/DMA/timer/I2S detail を backend 内部へ隔離。
- underrun、silent prefill、suspend/resume 測定。

Phase 6: asset converter integration

- WAV to PCM16 mono runtime-ready clip。
- sample rate/channel/duration/loop/memory budget validation。
- converter report と runtime asset header の接続。

Phase 7: SLDPCM4 experimental decoder

- build/asset flag 背後に実験 decoder を追加。
- codec experiment plan の metrics を収集。
- PCM16 fallback と loop validation を確認。

Phase 8: sequence/BGM v1

- `SequenceAsset`、BGM reserved bus の実体化。
- small instruments/wavetable、voice limits、bus volume、fade、priority stealing。
- SFX と BGM の fixed budget 共存を検証。

## 14. Open Questions

- v0 fixed sample rate を 16kHz、22.05kHz、または別値にするか。
- fixed block size と backend buffer depth をいくつにするか。
- PicoCalc の物理 audio backend は PWM、PIO、DMA、timer、I2S 相当のどれが適切か。
- mixer/backend を CPU0/CPU1 のどちらに置くか。VM との干渉をどう測るか。
- PSRAM/SD から asset を読む場合、preload、cache、resident 化、stream 禁止をどう分けるか。
- sequence v1 の最大 voice 数、instrument 数、event density をどう設定するか。
- SLDPCM4 を v1 stable に昇格するか、用途限定 experimental に留めるか、ADPCM4 を優先するか。
- active audio が suspend を延期できるか、power policy が常に停止できるか。
- event queue full 時の normal app 可視性をどこまで保証するか。

## 15. Design Conclusion

KotoAudio v0 は、PCM16 mono SFX を固定予算で安全に鳴らすための小さな audio
runtime として始める。通常アプリは source を要求し、completion/drop/underrun を
event queue と counters で観測する。mixer、decoder、backend、hardware detail は
KotoAudio が所有する。

v1 では sequence/BGM、bus、fade、priority stealing、codec 採否を広げる。ただし
stream、stereo、高品質 DSP、app callback、raw backend access は初期設計の中心にしない。
mock/simulator/PicoCalc backend は同じ mixer-facing API を共有し、diagnostics を最初から
持つことで、音切れや drop を見える失敗として扱う。
