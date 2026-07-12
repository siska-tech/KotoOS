# KotoAudio v0 Task Breakdown

## 0. Scope

この文書は、`docs/design/KOTO_AUDIO_ARCHITECTURE.md`、
`docs/design/KOTO_AUDIO_REQUIREMENTS.md`、
`docs/design/KOTO_AUDIO_CODEC_POLICY.md` に基づき、KotoAudio v0 の実装作業を
小さなチケットへ分解する。

v0 の中心は PCM16 mono SFX clip である。SLDPCM4 は v0 required ではなく、
experimental extension point として後段に回す。sequence/BGM、stream、PicoCalc 実機
backend は v0 最小経路の外側に置き、mock backend で source lifecycle、event queue、
counters、fixed block mixer を先に成立させる。

通常アプリは raw backend、PWM、PIO、DMA、timer、backend buffer へ直接アクセスしない。
completion は callback ではなく event queue で観測する。diagnostics/counters は最初から
持ち、drop、queue full、underrun、late mix を見える失敗として扱う。

## 1. Milestone Overview

| Milestone | Name | Outcome |
|---|---|---|
| M0 | crate skeleton | `koto-audio` crate と v0 required module 境界を作る。実音声出力は持たない。 |
| M1 | source lifecycle | `SourceId`、bounded queue、active slot、completion/drop/stop の state transition を実装する。 |
| M2 | PCM16 clip decode path | runtime-ready PCM16 mono clip asset を検証し、decoder が PCM16 sample stream を返す。 |
| M3 | mock backend | deterministic な backend abstraction を作り、submitted block、queue full、underrun をテスト可能にする。 |
| M4 | fixed block mixer | fixed sample rate/block size の mono mixer が active source から `MixerBlock` を生成する。 |
| M5 | AudioService API | source admission、mixer tick、backend submit、event/counter を `AudioService` で統合する。 |
| M6 | hostcall adapter | normal/system/debug hostcall 境界を `AudioService` へ接続し、normal app の権限を制限する。 |
| M7 | simulator backend | mock と同じ API で simulator output または inspection 用 backend を追加する。 |
| M8 | PicoCalc backend experiment | PWM/PIO/DMA/timer/I2S 候補を backend 内部に閉じた実機実験として進める。 |
| M9 | asset converter integration | WAV to PCM16 mono runtime-ready clip と validation report を runtime header へ接続する。 |
| M10 | SLDPCM4 experimental | build/asset flag 背後に SLDPCM4 decoder 実験を追加する。v0 compatibility には要求しない。 |
| M11 | sequence/BGM v1 | `SequenceAsset`、BGM bus、bus volume、fade、voice limits を v1 として実体化する。 |

## 2. Task List

### KA-M0-001: Create koto-audio crate skeleton

- Goal: `crates/koto-audio` を追加し、v0 required modules の空の境界を作る。
- Files/modules likely touched: `Cargo.toml`, `crates/koto-audio/Cargo.toml`, `crates/koto-audio/src/lib.rs`, `service.rs`, `source.rs`, `clip.rs`, `decoder.rs`, `mixer.rs`, `backend.rs`, `event.rs`, `counters.rs`, `policy.rs`, `asset.rs`, `hostcall.rs`.
- Dependencies: none.
- Acceptance criteria: crate が workspace に参加する。public re-export は core type のみ。実機 backend、SLDPCM4、sequence、stream の実装を含まない。
- Test strategy: `cargo check`。crate visibility の compile test。
- Non-goals: real audio output、PicoCalc backend、asset converter、hostcall ABI の確定。

### KA-M0-002: Define v0 policy and static limits

- Goal: sample rate/block size 未確定を表現できる `AudioPolicy` と fixed limits の初期構造を定義する。
- Files/modules likely touched: `policy.rs`, `lib.rs`, `counters.rs`.
- Dependencies: KA-M0-001.
- Acceptance criteria: SFX source count、source queue depth、event queue depth、volume bounds、drop/steal policy の型がある。sample rate と block size は build/policy で固定可能だが、値の最終決定を要求しない。
- Test strategy: unit tests for default policy invariants and invalid limit rejection.
- Non-goals: 実機測定、dynamic policy変更、system hostcallからの backend policy 制御。

### KA-M0-003: Define public core result and error types

- Goal: play/stop/poll/query で使う status、error、reason 型を定義する。
- Files/modules likely touched: `lib.rs`, `service.rs`, `event.rs`, `asset.rs`.
- Dependencies: KA-M0-001.
- Acceptance criteria: admission failure、queue full、malformed asset、unsupported codec、backend unavailable を区別できる。backend raw detail は公開 error に含めない。
- Test strategy: compile tests and debug formatting snapshot if useful.
- Non-goals: hostcall numeric ABI mapping、hardware error code の公開。

### KA-M1-001: Implement SourceId with generation

- Goal: stale id 誤操作を避ける不透明な `SourceId` を実装する。
- Files/modules likely touched: `source.rs`, `event.rs`.
- Dependencies: KA-M0-001.
- Acceptance criteria: slot index が直接 public API から見えない。generation により解放後の古い id を拒否できる。
- Test strategy: allocate/free/reallocate unit tests、stale id stop/volume rejection tests.
- Non-goals: cross-process security token、persistent id、debug raw slot exposure。

### KA-M1-002: Implement bounded source queue

- Goal: accepted clip request を bounded queue に積み、満杯時は即時 status と counter reason を返す。
- Files/modules likely touched: `source.rs`, `policy.rs`, `counters.rs`.
- Dependencies: KA-M0-002, KA-M1-001.
- Acceptance criteria: queue depth を超えた request は block しない。`queue_full_count` と `dropped_source_count` を増やすための hook がある。
- Test strategy: queue capacity boundary tests、FIFO order tests、queue full tests.
- Non-goals: priority stealing、mixer昇格、event queue配送。

### KA-M1-003: Implement active source slots and lifecycle states

- Goal: queued、playing、stopping、completed、dropped、stolen、error の state transition を管理する。
- Files/modules likely touched: `source.rs`, `event.rs`.
- Dependencies: KA-M1-001, KA-M1-002.
- Acceptance criteria: queued source を active slot へ昇格できる。one-shot completion、stop、drop が logical state として表現される。
- Test strategy: lifecycle unit tests、stop before activation、stop while playing、completion transition tests.
- Non-goals: actual decoding、mixing、loop audio correctness、backend submit。

### KA-M1-004: Add source priority admission hooks

- Goal: priority と future voice stealing のための admission policy hook を入れる。
- Files/modules likely touched: `source.rs`, `policy.rs`, `counters.rs`.
- Dependencies: KA-M1-003.
- Acceptance criteria: v0 では reject/drop を基本にし、steal path は明示的に分離される。steal counter/event を後で接続できる。
- Test strategy: low priority rejection tests、policy branch unit tests.
- Non-goals: sophisticated priority algorithm、BGM/SFX bus priority、system focus policy。

### KA-M2-001: Define ClipAsset and runtime header validation

- Goal: PCM16 mono at mixer rate を表す runtime-ready `ClipAsset` metadata を定義し、基本 validation を行う。
- Files/modules likely touched: `clip.rs`, `asset.rs`, `codec/pcm16.rs`, `policy.rs`.
- Dependencies: KA-M0-002, KA-M0-003.
- Acceptance criteria: codec id、sample rate、channels、sample count、loop metadata、placement hint を持つ。v0 required は PCM16 mono のみ。mismatched sample rate、non-mono、invalid loop は malformed/unsupported として拒否される。
- Test strategy: valid/invalid asset header tests、loop range tests.
- Non-goals: WAV parsing、runtime resampling、SLDPCM4 metadata実装、SD streaming。

### KA-M2-002: Implement Decoder trait and PCM16 decoder

- Goal: codec-specific payload を PCM16 mono sample stream として mixer に渡す最小 decoder contract を作る。
- Files/modules likely touched: `decoder.rs`, `codec/pcm16.rs`, `clip.rs`.
- Dependencies: KA-M2-001.
- Acceptance criteria: PCM16 decoder state は sample cursor のみ。end-of-clip と simple loop を返せる。codec internals は normal app API へ露出しない。
- Test strategy: decode exact sample tests、odd/even sample counts、end-of-clip、whole-clip loop tests.
- Non-goals: PCM8、SLDPCM4、ADPCM4、seek、pitch shift。

### KA-M2-003: Wire decoder state into SourceSlot

- Goal: active source slot が decoder state を所有し、source lifecycle と decode cursor を一緒に進められるようにする。
- Files/modules likely touched: `source.rs`, `decoder.rs`, `clip.rs`.
- Dependencies: KA-M1-003, KA-M2-002.
- Acceptance criteria: active slot 初期化時に decoder が作られる。completion と decoder end が source state に反映される。
- Test strategy: queued-to-active-with-decoder tests、completion source id matching tests.
- Non-goals: mixing output、backend output、compressed decoder state。

### KA-M3-001: Define AudioBackend trait and MixerBlock contract

- Goal: mixer-facing backend API と fixed block 型を定義する。
- Files/modules likely touched: `backend.rs`, `mixer.rs`, `policy.rs`.
- Dependencies: KA-M0-002.
- Acceptance criteria: `start`、`stop`、`submit_block`、`query_state`、`suspend`、`resume` 相当の境界がある。PWM/PIO/DMA/timer detail は trait の外へ出ない。
- Test strategy: compile tests using a dummy backend.
- Non-goals: real device output、DMA queue、simulator audio device。

### KA-M3-002: Implement deterministic mock backend

- Goal: submitted blocks と backend state を記録する mock backend を実装する。
- Files/modules likely touched: `backend/mock.rs`, `backend.rs`, `mixer.rs`.
- Dependencies: KA-M3-001.
- Acceptance criteria: start/stop state transition、submitted block recording、manual underrun injection、submit failure simulation ができる。
- Test strategy: mock backend unit tests、forced underrun tests.
- Non-goals: audio playback、wall-clock timing、PicoCalc register access。

### KA-M3-003: Add backend counters/report hooks

- Goal: backend underrun、restart、submit failure を counters/event候補へ報告する hook を作る。
- Files/modules likely touched: `backend.rs`, `backend/mock.rs`, `counters.rs`, `event.rs`.
- Dependencies: KA-M3-002.
- Acceptance criteria: mock backend から underrun を service 側へ伝えられる。`underrun_count` と `backend_restart_count` を更新できる。
- Test strategy: forced underrun counter tests、restart count tests.
- Non-goals: debug-only detailed backend state、hardware subtype counter。

### KA-M4-001: Implement fixed mono MixerBlock generation

- Goal: fixed block size の mono PCM16 相当 block を生成する mixer を実装する。
- Files/modules likely touched: `mixer.rs`, `source.rs`, `decoder.rs`.
- Dependencies: KA-M2-003, KA-M3-001.
- Acceptance criteria: active source がない場合は silence block を出す。PCM16 source がある場合は non-silent block を出せる。block length は policy と一致する。
- Test strategy: silence tests、single clip block tests、short clip completion at block boundary tests.
- Non-goals: real-time scheduling、backend timing、PicoCalc output。

### KA-M4-002: Add integer volume and saturation

- Goal: source/app/master volume を整数演算で適用し、signed 32bit accumulator から PCM16 相当へ saturation する。
- Files/modules likely touched: `mixer.rs`, `service.rs`, `policy.rs`.
- Dependencies: KA-M4-001.
- Acceptance criteria: zero volume は silence。複数 source の加算が overflow しない。final output の clipping policy が deterministic。
- Test strategy: volume scale tests、multi-source saturation tests、master/app/source composition tests.
- Non-goals: floating point DSP、fade、bus volume、stereo。

### KA-M4-003: Emit completion candidates from mixer

- Goal: mixer が source 終端、loop 終了、stop、error を event queue へ渡す候補として返す。
- Files/modules likely touched: `mixer.rs`, `source.rs`, `event.rs`.
- Dependencies: KA-M4-001, KA-M1-003.
- Acceptance criteria: one-shot completion が block generation 後に観測できる。event は ISR/backend 文脈で user callback を呼ばない設計になっている。
- Test strategy: one-shot completion event candidate tests、finite loop completion tests.
- Non-goals: hostcall poll integration、cross-app filtering。

### KA-M4-004: Add mixer diagnostics counters

- Goal: active/queued count、late mix、max mix time、silence fill を counter snapshot に反映する。
- Files/modules likely touched: `mixer.rs`, `counters.rs`, `service.rs`.
- Dependencies: KA-M4-001.
- Acceptance criteria: `active_source_count`、`queued_source_count`、`late_mix_count`、`max_mix_time` の更新 hook がある。time unit は実装内で一貫している。
- Test strategy: deterministic counter update tests with injectable clock/tick source.
- Non-goals: cycle-accurate RP2040 measurement、debug graphing。

### KA-M5-001: Implement AudioService construction and lifecycle

- Goal: policy、source manager、mixer、backend、event queue、counters を所有する `AudioService` を作る。
- Files/modules likely touched: `service.rs`, `policy.rs`, `source.rs`, `mixer.rs`, `backend.rs`, `event.rs`, `counters.rs`.
- Dependencies: KA-M1-003, KA-M3-002, KA-M4-001.
- Acceptance criteria: service start/stop/tick の基本 lifecycle がある。backend can be swapped through the same abstraction.
- Test strategy: service construction tests、backend swap compile/unit tests.
- Non-goals: hostcall ABI、system power integration complete、real audio thread。

### KA-M5-002: Implement AudioService::play_clip

- Goal: asset validation、source admission、queue insertion、`SourceId` return を統合する。
- Files/modules likely touched: `service.rs`, `clip.rs`, `asset.rs`, `source.rs`, `counters.rs`.
- Dependencies: KA-M2-001, KA-M1-002, KA-M5-001.
- Acceptance criteria: valid PCM16 clip は `SourceId` を返す。invalid asset は admitted されず `malformed_asset_count` を増やす。queue full は immediate status と counter に反映される。
- Test strategy: play valid clip tests、malformed asset tests、queue full tests.
- Non-goals: sequence playback、stream playback、compressed codec selection。

### KA-M5-003: Implement tick-to-backend path

- Goal: `AudioService` tick が mixer block を生成し、backend へ submit し、failure を counter/event に反映する。
- Files/modules likely touched: `service.rs`, `mixer.rs`, `backend.rs`, `event.rs`, `counters.rs`.
- Dependencies: KA-M4-003, KA-M3-003, KA-M5-001.
- Acceptance criteria: PCM16 clip can be played through mock backend. submitted block が mock backend に残る。backend submit failure は underrun/drop 相当の observable state になる。
- Test strategy: end-to-end mock backend tests、forced submit failure tests.
- Non-goals: real-time periodic scheduling、simulator audio device、PicoCalc backend。

### KA-M5-004: Implement event queue and poll API

- Goal: completion/drop/stop/stolen/error/underrun を bounded event queue に入れ、poll で取得できるようにする。
- Files/modules likely touched: `event.rs`, `service.rs`, `source.rs`, `counters.rs`.
- Dependencies: KA-M4-003, KA-M5-003.
- Acceptance criteria: source completion event can be polled. event queue full 時は event drop counter/debug counter が増える。event は `SourceId` と reason を含み backend pointer を含まない。
- Test strategy: completion poll tests、event ordering tests、event queue full tests.
- Non-goals: user callback、ISR delivery、cross-process event transport。

### KA-M5-005: Implement stop and volume service APIs

- Goal: `stop`、`set_source_volume`、`set_app_volume` の normal app 向け core API を実装する。
- Files/modules likely touched: `service.rs`, `source.rs`, `mixer.rs`, `event.rs`.
- Dependencies: KA-M5-002, KA-M5-004.
- Acceptance criteria: source id scope で stop できる。stale id は拒否される。source/app volume が mixer 出力に反映される。
- Test strategy: stop event tests、stale id tests、volume output tests.
- Non-goals: master volume hostcall、fade、audio focus。

### KA-M5-006: Implement counter snapshot API

- Goal: normal app に許可された counters を snapshot として返す。
- Files/modules likely touched: `counters.rs`, `service.rs`, `hostcall.rs`.
- Dependencies: KA-M5-004.
- Acceptance criteria: active/queued/dropped/stolen/underrun/late mix/max mix time/backend restart/queue full/malformed asset を保持できる。normal snapshot は raw backend state を含まない。
- Test strategy: counter snapshot tests、normal vs debug visibility tests.
- Non-goals: persistent telemetry、visual dashboard。

### KA-M6-001: Define normal hostcall adapter surface

- Goal: normal app が使える logical audio hostcall を thin adapter として定義する。
- Files/modules likely touched: `hostcall.rs`, `service.rs`.
- Dependencies: KA-M5-002, KA-M5-004, KA-M5-006.
- Acceptance criteria: `play_clip`、`play_sequence` placeholder、`stop`、`set_source_volume`、`set_app_volume`、`poll_audio_event`、`query_audio_counters` が service へ委譲される。normal app から backend policy や hardware detail に触れない。
- Test strategy: API surface tests、unsupported `play_sequence` tests.
- Non-goals: stable syscall numbers、VM integration complete、system/debug hostcalls。

### KA-M6-002: Define system/debug hostcall placeholders

- Goal: system/debug の広い権限を normal から分離する placeholder を用意する。
- Files/modules likely touched: `hostcall.rs`, `service.rs`, `counters.rs`.
- Dependencies: KA-M6-001.
- Acceptance criteria: master volume、mute、audio focus、backend policy、debug dumps は normal API と別 namespace または feature gate にある。
- Test strategy: compile/API visibility tests.
- Non-goals: full power manager integration、real backend selection UI、hardware dump。

### KA-M6-003: Add raw backend access negative tests

- Goal: normal app API から PWM/PIO/DMA/timer/backend buffer に到達できないことをテストで固定する。
- Files/modules likely touched: public API tests, `hostcall.rs`, `backend.rs`.
- Dependencies: KA-M6-001, KA-M3-001.
- Acceptance criteria: public exports に raw backend detail が出ない。normal hostcall は backend pointer、buffer pointer、timer/DMA handle を返さない。
- Test strategy: compile-fail or API surface tests.
- Non-goals: formal security proof、kernel sandbox enforcement。

### KA-M7-001: Add simulator backend using AudioBackend API

- Goal: desktop/simulator で buffer inspection 可能な backend を mock と同じ API で追加する。
- Files/modules likely touched: `backend/sim.rs`, `backend.rs`, `service.rs`.
- Dependencies: KA-M3-001, KA-M5-001.
- Acceptance criteria: simulator backend can be swapped without changing `AudioService` call sites. buffer fill/state inspection が debug 経由で可能。
- Test strategy: backend conformance tests shared with mock.
- Non-goals: production audio latency tuning、PicoCalc hardware、streaming。

### KA-M7-002: Add KotoBlocks SFX simulator acceptance

- Goal: VM を block せず `play_clip` 連打、completion poll、queue full/counter を simulator で検証する。
- Files/modules likely touched: integration tests, simulator fixture, docs.
- Dependencies: KA-M7-001, KA-M5-004, KA-M5-006.
- Acceptance criteria: repeated SFX requests do not wait for mixer completion. queue full/drop counters are updated. completion events remain pollable.
- Test strategy: integration test with small PCM16 fixture clips.
- Non-goals: real game asset pack、PicoCalc performance measurement。

### KA-M8-001: Create PicoCalc backend experiment boundary

- Goal: PicoCalc 実機 backend を実験 module として隔離し、raw hardware detail を backend 内部に閉じる。
- Files/modules likely touched: `backend/picocalc.rs`, backend feature config, experiment docs.
- Dependencies: KA-M3-001, KA-M5-001.
- Acceptance criteria: feature flag または target cfg 背後にある。PWM/PIO/DMA/timer/I2S detail は public API に漏れない。
- Test strategy: target-gated compile checks where possible.
- Non-goals: final backend selection、stable timing、production audio quality。

### KA-M8-002: Measure PicoCalc backend candidates

- Goal: sample rate、block size、buffer depth、CPU placement、underrun behavior を実機で測る。
- Files/modules likely touched: experiment harness, `backend/picocalc.rs`, measurement docs.
- Dependencies: KA-M8-001.
- Acceptance criteria: 16kHz/22.05kHz 候補、128/256 block 候補、silent prefill、underrun report の測定結果が残る。
- Test strategy: hardware measurement runs, counter logs.
- Non-goals: public API changes、SLDPCM4採否、sequence/BGM。

### KA-M9-001: Define runtime-ready clip asset format

- Goal: converter と runtime が共有する PCM16 mono clip header を確定する。
- Files/modules likely touched: `asset.rs`, converter crate/module, docs.
- Dependencies: KA-M2-001.
- Acceptance criteria: format version、codec id、sample rate、channels、duration/sample count、loop metadata、budget hints を持つ。
- Test strategy: header encode/decode tests、invalid header tests.
- Non-goals: compressed codec bitstream、sequence format。

### KA-M9-002: Implement WAV to PCM16 mono converter integration

- Goal: PC 側 converter が WAV を mixer rate PCM16 mono runtime-ready clip に変換または拒否する。
- Files/modules likely touched: converter crate/module, `asset.rs`, docs.
- Dependencies: KA-M9-001, sample rate decision from KA-M8-002 or build config.
- Acceptance criteria: sample rate、channels、duration、loop point、memory budget を検証する。runtime arbitrary resampling はしない。
- Test strategy: converter fixture tests、invalid WAV/loop tests、golden PCM output tests.
- Non-goals: SLDPCM4 encode、sequence conversion、runtime WAV parsing。

### KA-M9-003: Add converter report and runtime validation alignment

- Goal: converter report と runtime validation のエラー分類を揃える。
- Files/modules likely touched: converter report module, `asset.rs`, `counters.rs`.
- Dependencies: KA-M9-002.
- Acceptance criteria: report に codec、source WAV params、output sample rate、sample count、asset bytes、memory budget、loop validation を含める。runtime で不正 asset は `malformed_asset_count` に反映される。
- Test strategy: report snapshot tests、runtime rejects converter-negative fixtures.
- Non-goals: subjective audio QA tool、waveform UI。

### KA-M10-001: Add SLDPCM4 experimental decoder trait hook

- Goal: SLDPCM4 を build/asset flag 背後の codec extension point として追加する。
- Files/modules likely touched: `decoder.rs`, `codec/sldpcm4.rs`, `asset.rs`.
- Dependencies: KA-M2-002, KA-M9-001.
- Acceptance criteria: PCM16 path は default required のまま。SLDPCM4 disabled build では unsupported codec として拒否される。
- Test strategy: feature-gated compile tests、unsupported codec tests.
- Non-goals: v0 required support、normal app codec selection、ADPCM4。

### KA-M10-002: Implement SLDPCM4 loop and malformed payload tests

- Goal: experimental SLDPCM4 の loop predictor seed、nibble parity、payload bounds を検証する。
- Files/modules likely touched: `codec/sldpcm4.rs`, `clip.rs`, `asset.rs`.
- Dependencies: KA-M10-001.
- Acceptance criteria: loop state は source slot 内に閉じる。payload overrun と invalid loop metadata は error/counter に反映される。
- Test strategy: feature-gated decoder tests、loop transition tests、malformed payload tests.
- Non-goals: promotion to stable、asset corpus quality gate。

### KA-M10-003: Add SLDPCM4 converter experiment report

- Goal: peak error、RMS error、saturation count、loop transition error、fallback rate を測定する。
- Files/modules likely touched: converter experimental codec module, experiment docs.
- Dependencies: KA-M10-001, KA-M9-003.
- Acceptance criteria: PCM16 fallback が可能。measurement report が SLDPCM4 採否判断に使える。
- Test strategy: representative asset corpus experiment.
- Non-goals: v0 default codec、ABI freeze。

### KA-M11-001: Define SequenceAsset v1 placeholder

- Goal: v0 では unsupported としつつ、v1 の `SequenceAsset` 境界を設計する。
- Files/modules likely touched: `sequence.rs`, `hostcall.rs`, docs.
- Dependencies: KA-M6-001.
- Acceptance criteria: `play_sequence` は v0 で unsupported/placeholder を返す。v0 PCM16 clip path に影響しない。
- Test strategy: unsupported hostcall tests.
- Non-goals: music playback、instrument interpreter、BGM bus実装。

### KA-M11-002: Implement BGM bus and sequence runtime for v1

- Goal: v1 で SFX と BGM が fixed budget 内で共存できる runtime を追加する。
- Files/modules likely touched: `sequence.rs`, `bus.rs`, `mixer.rs`, `policy.rs`, converter sequence module.
- Dependencies: KA-M11-001, v0 acceptance complete.
- Acceptance criteria: sequence voice limits、BGM reserved bus、bus volume、loop、priority stealing が定義される。
- Test strategy: sequence playback tests、SFX+BGM budget tests、voice stealing tests.
- Non-goals: v0 scope、streaming BGM、high quality stereo。

## 3. First Implementation Slice

最初の PR/commit は M0 の最小範囲に限定する。

Recommended first slice:

- crate skeleton を作る。
- `lib.rs` と v0 required module files を追加する。
- core types only: `AudioPolicy`、`SourceId`、basic status/error/event/counter type の skeleton。
- no real audio output。
- no PicoCalc backend。
- no SLDPCM4。
- no sequence。
- no stream。
- no hostcall ABI numbers。

First slice acceptance:

- workspace の `cargo check` が通る。
- public API は logical audio type のみで、raw backend/hardware detail を re-export しない。
- TODO は milestone/task id に紐づく最小限に留める。
- PCM16 decoder、mock backend、mixer の実ロジックは次 slice へ回す。

## 4. v0 Acceptance Path

v0 として満たすべき最終状態:

- PCM16 clip can be played through mock backend.
- source completion event can be polled.
- queue full/drop counters are updated.
- mixer emits fixed blocks.
- normal app cannot access raw backend.
- backend can be swapped.

Acceptance sequence:

1. M0 で crate skeleton と core type を作る。
2. M1 で bounded source lifecycle を作り、queue full/drop を state と counter hook で表現する。
3. M2 で PCM16 mono clip validation と decoder を作る。
4. M3 で mock backend を作り、backend abstraction を固定する。
5. M4 で fixed block mixer を作り、PCM16 source から block を生成する。
6. M5 で `AudioService` が play/tick/poll/counter を統合し、mock backend end-to-end test を通す。
7. M6 で hostcall adapter を追加し、normal app に raw backend access がないことを API surface test で固定する。

v0 では simulator backend、PicoCalc backend、asset converter integration は重要だが、最小 acceptance は
mock backend で成立させる。実機依存は v0 core の後段に置き、API と lifecycle の形を先に固める。

## 5. Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| sample rate 未確定 | asset format、mixer cost、backend timing、converter output に影響する。 | policy/build config で固定値を差し替え可能にし、PicoCalc 測定前は PCM16 path の構造だけを固める。 |
| block size 未確定 | latency、event timing、backend buffer depth、mix deadline に影響する。 | `MixerBlock` を policy 由来の固定長として扱い、128/256 候補の測定を M8 に分離する。 |
| PicoCalc backend 未確定 | hardware output、underrun、pop noise、power hooks が未確定。 | mock backend を先行し、PicoCalc は `AudioBackend` 内部実験として M8 へ後置する。 |
| CPU0/CPU1 placement 未確定 | VM と mixer/backend の干渉、deadline miss に影響する。 | scheduler/thread/core placement を public API に漏らさず、diagnostics counter で late mix と max mix time を測る。 |
| PSRAM/SD asset placement 未確定 | asset read latency、RAM budget、stream禁止範囲に影響する。 | v0 は memory-resident PCM16 clip を基本にし、placement hint は metadata に留める。SD stream は後回し。 |
| SLDPCM4 採否未確定 | codec ABI、loop state、converter quality gate に影響する。 | PCM16 を required/golden path にし、SLDPCM4 は feature-gated experimental として M10 に分離する。 |

