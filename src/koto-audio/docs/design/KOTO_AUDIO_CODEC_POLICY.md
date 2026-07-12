# KotoAudio Clip Codec Policy

## 0. Scope

この文書は、KotoAudioにおけるshort clip/SFX向けcodec方針を定義する。KotoAudioは小型ゲーム機向けbounded audio runtimeであり、v0では短い効果音、UI音、短いジングルを主対象にする。

KotoAudioはPC側asset converterでWAVをruntime-ready assetへ変換する。runtimeはarbitrary resamplingを行わず、通常アプリはcodec、mixer backend、PWM、PIO、DMA、timerを直接操作しない。アプリはclip再生を要求し、KotoAudioがsource、decoder、mixer、backendを所有する。

基準形式はPCM16 monoである。PCM8、ADPCM4、SLDPCM4は容量、CPU、音質、loop安全性のtradeoffとして扱う。SLDPCM4は、前回復元sampleとの差分を4bitの対数ステップへ量子化するKotoAudio独自候補として整理する。P/ECE互換codecは目指さず、既存SLDPCM実装のCコードをそのまま移植しない。

参考: `https://github.com/yosi2112/SLDPCM/tree/master` はSLDPCMが高速decodeを目的としたcodecであること、入力がsigned 16bit mono/stereoであることを確認するための参考に留める。KotoAudioのbitstream、decode table、loop規則、asset validationは独自仕様として定義する。

## 1. Codec Goals

Clip codecの目標は、音声品質だけでなく、runtime予算を壊さないことである。

- Bounded decode: 1 sampleあたりのdecode costを固定し、branch、table参照、状態更新を小さく保つ。
- Runtime-ready asset: sample rate、channel数、loop点、codec設定、予算情報はPC側converterで検証済みにする。
- PCM16 as reference: converter内部評価、golden output、mixer入力の基準をsigned PCM16に置く。
- No runtime arbitrary resampling: asset sample rateはmixer rateと一致させる。pitch変更や任意rate変換はv0 clip codecの責務にしない。
- App isolation: 通常アプリはcodec選択、decode table、backend formatを直接指定しない。asset metadataと再生policyだけをKotoAudioへ渡す。
- Loop safety: loop点はsample境界で定義し、差分codecでもloop時のdecoder stateを明示する。
- Measurable adoption: 圧縮codecは容量削減、decode時間、音質劣化、loop artifact、converter失敗率を計測して採否を決める。

## 2. Codec Comparison

| Codec | Size | Decode cost | Quality | Loop behavior | v0 suitability |
|---|---:|---:|---|---|---|
| PCM16 | 16 bit/sample | 最小 | 基準品質。converter出力のgolden形式 | sample位置だけでloop可能 | 必須。最初の基準形式 |
| PCM8 | 8 bit/sample | 最小 | 小音量、余韻、環境音で量子化ノイズが目立つ | sample位置だけでloop可能 | 任意。容量節約用だがPCM16より優先しない |
| ADPCM4 | 4 bit/sample | 低から中 | adaptive stepなら広い音量に強いが、state管理が必要 | predictor/step indexのloop stateが必要 | 候補。既存知見は多いが仕様をKotoAudio独自に固定する必要がある |
| SLDPCM4 | 4 bit/sample | 低 | fixed logarithmic deltaのため単純で速い。急峻なtransient、大振幅低域、静音付近のノイズは要測定 | previous sampleとnibble位置のloop stateが必要 | 実験候補。v0必須にはしない |

### PCM16

PCM16はKotoAudio clipの基準形式である。mixerへ渡す前のsource sampleはsigned 16bit monoとして扱う。短いSFX中心のv0では、容量より確実性と実装単純性を優先し、PCM16だけで成立する最小runtimeを先に作る。

### PCM8

PCM8は容量を半分にでき、decodeも符号拡張または中心値補正だけで済む。ただしKotoAudioの基準形式をPCM16に置くため、PCM8は「低品質を許容するasset profile」として扱う。v0で入れる場合も、PCM16 mixerへ展開してからvolumeとmixを行う。

### ADPCM4

ADPCM4は4bit/sampleで、adaptive stepを持てば多様な音に対応しやすい。一方でpredictor、step index、block restart、loop checkpointなどの仕様が増える。KotoAudioではP/ECE互換ADPCMを採用しない。ADPCM4を採用する場合は、decode table、step update、block header、loop stateをKotoAudio仕様として別途固定する。

### SLDPCM4

SLDPCM4は「sampleそのもの」ではなく「前回復元sampleとの差分」を4bitで表す。差分の大きさは線形ではなく対数ステップにする。KotoAudio案ではadaptive step indexを持たない固定table方式を第一候補にし、decodeを `previous_sample + delta_table[nibble]` のsaturating updateに近い形へ抑える。

固定table方式は速く、decoder stateが小さい。一方でclipごとの音量、帯域、transientに対して最適tableが変わりうるため、asset converterで品質判定し、PCM16またはADPCM4へfallbackできることが必須である。

## 3. SLDPCM4 Format Proposal

SLDPCM4はKotoAudio独自のclip payload codecであり、互換対象を持たない。

Asset metadata:

| Field | Requirement |
|---|---|
| codec id | `SLDPCM4`。KotoAudio内部codec IDで識別する。 |
| version | 初期仕様は`1`。decode tableやpackingを変える場合はversionを上げる。 |
| sample rate | mixer sample rateと一致していること。runtime resamplingは禁止。 |
| channels | v0はmonoのみ。stereoは非目標。 |
| sample count | 復元後PCM16 sample数。nibble数ではない。 |
| initial sample | sample 0の前に置くpredictor seed。通常は0または先頭sample近似値。 |
| step shift | decode tableの基準deltaを左shiftするclip単位係数。 |
| payload packing | 1 byteに2 sample分のnibbleを格納する。高nibbleが先、低nibbleが次。 |
| loop metadata | loop有効時はloop start、loop end、loop count、loop predictor seed、loop nibble parityを持つ。 |
| restart interval | v0では任意。長いclipやseek対応が必要になった場合、固定sample間隔のpredictor checkpointを追加できる。 |

Decode model:

- decoderは`previous_sample`をsigned 16bitとして持つ。
- 各nibbleをdecode tableでsigned deltaへ変換する。
- `previous_sample + delta` をsigned 16bit範囲へsaturateし、復元sampleと次の`previous_sample`にする。
- asset末尾でsample countに達したら停止またはloop処理へ移る。padding nibbleは無視する。
- v0 decoderはseek、reverse、runtime table変更を提供しない。

Encoder model:

- encoderは入力WAVをPCM16 mono、mixer sample rateへPC側で変換済みにする。
- encoderは各target sampleに最も近い復元sampleを作るnibbleを選ぶ。
- 量子化誤差は次sampleへ持ち越してよいが、その方式はconverter内部実装であり、runtime bitstream仕様に含めない。
- `step shift`はclip単位で探索し、品質閾値に届かない場合はSLDPCM4を不採用にする。

## 4. SLDPCM4 Decode Table

KotoAudio experimental SLDPCM4は固定の `standard16` decode tableを持つ。このtableは16bit PCM向けのKotoAudio独自tableであり、既存SLDPCM tableやbitstreamとの互換を意味しない。

| Nibble | Delta | Meaning |
|---:|---:|---|
| `0xF` | +16384 | largest positive step |
| `0xE` | +8192 | positive step |
| `0xD` | +4096 | positive step |
| `0xC` | +2048 | positive step |
| `0xB` | +1024 | positive step |
| `0xA` | +512 | positive step |
| `0x9` | +256 | small positive step |
| `0x8` | 0 | hold |
| `0x7` | -256 | small negative step |
| `0x6` | -512 | negative step |
| `0x5` | -1024 | negative step |
| `0x4` | -2048 | negative step |
| `0x3` | -4096 | negative step |
| `0x2` | -8192 | negative step |
| `0x1` | -16384 | negative step |
| `0x0` | -32768 | largest negative step |

Table policy:

- `0x8` is the canonical zero delta.
- Decoder must not derive the table from external app input. The table is codec-version-owned.
- Converter and runtime must use the same `standard16` table.

The table is intentionally simple. If measurement shows unacceptable quality, KotoAudio should prefer PCM16 or a separately specified ADPCM4 over mutating SLDPCM4 silently.

## 5. Encoder/Decoder Responsibilities

PC asset converter responsibilities:

- Read WAV and normalize it into signed PCM16 mono.
- Convert sample rate to the selected KotoAudio mixer rate on PC, never on runtime.
- Reject unsupported channel counts, bit depths, durations, loop points, and sample rates before producing the asset.
- Choose PCM16, optional PCM8, ADPCM4, or SLDPCM4 according to policy, quality threshold, and budget target.
- For SLDPCM4, search `step_shift`, encode nibbles, generate predictor seeds, and compute decoded PCM16 for validation.
- Measure quality against the PCM16 reference using at least peak error, RMS error, and clipping/saturation count.
- Verify loop boundaries by decoding across loop transitions.
- Emit metadata that allows runtime to decode without guessing.

Runtime decoder responsibilities:

- Trust only validated runtime-ready assets.
- Decode at mixer sample rate without resampling.
- Keep source-local decoder state and never expose codec internals to normal apps.
- Produce signed PCM16 samples for the mixer.
- Handle end-of-clip and loop state exactly as metadata defines.
- Count malformed asset failures in debug builds, but avoid expensive runtime repair.

Normal app responsibilities:

- Select and play logical clip assets.
- Provide source metadata such as volume, priority, and loop request when allowed by the asset.
- Not select decode tables, codec parameters, or backend sample formats directly.

## 6. Loop Handling

Loop points are expressed in decoded PCM sample indices:

- `loop_start` is inclusive.
- `loop_end` is exclusive.
- `0 <= loop_start < loop_end <= sample_count`.
- v0 supports whole-clip loop and simple forward loop only.
- Ping-pong loop, crossfade loop, loop with runtime pitch shift, and stream-like refill are out of scope.

For PCM16 and PCM8, loop state is only the sample cursor.

For ADPCM4 and SLDPCM4, loop state includes decoder state. SLDPCM4 loop metadata must include:

| Field | Requirement |
|---|---|
| loop predictor seed | `previous_sample` value to install before decoding `loop_start`. |
| loop nibble offset | payload nibble index for `loop_start`. |
| loop nibble parity | whether `loop_start` begins at high or low nibble. |

Loop transition rule:

1. When the source cursor reaches `loop_end`, decrement finite loop count if applicable.
2. If another loop iteration remains, set payload cursor to `loop_start` nibble position.
3. Restore `previous_sample` from loop predictor seed.
4. Continue decode from `loop_start`.

The converter must compute loop predictor seed by decoding from a legal restart point or by storing an explicit checkpoint. Runtime must not scan from sample 0 on every loop.

## 7. Source State and Decoder State

Source state is codec-independent and owned by KotoAudio:

| State | Meaning |
|---|---|
| queued | Play request accepted but not yet mixed. |
| playing | Source is active and contributes samples. |
| paused | Source cursor and decoder state are retained but not advanced. |
| stopping | Stop or fade-out requested. |
| completed | One-shot or finite loop completed normally. |
| stolen | Source was replaced by priority policy. |
| dropped | Source could not be admitted due to queue or budget limits. |
| error | Asset or decoder error detected. |

Decoder state is codec-specific and hidden inside the active source slot.

PCM16/PCM8 decoder state:

- sample cursor.

SLDPCM4 decoder state:

- decoded sample cursor.
- payload byte pointer or byte offset.
- nibble parity.
- `previous_sample`.
- `step_shift`.
- remaining loop count or infinite loop marker.

ADPCM4 decoder state, if adopted:

- decoded sample cursor.
- payload cursor and nibble parity.
- predictor.
- step index or equivalent adaptive state.
- block/restart state.
- remaining loop count or infinite loop marker.

The mixer advances source state and decoder state together. App-visible state query may report logical source state, but not predictor values, step index, raw payload offsets, or backend buffer positions.

## 8. Mixer Output Format

Codec decoders output signed PCM16 mono samples into the mixer path. The mixer may use wider accumulators internally.

Required mixer format policy:

- Source decoder output: signed PCM16.
- Source volume/app volume/master volume: applied after decode.
- Mix accumulator: at least signed 32bit for multiple active sources.
- Saturation: final mix is saturated or clipped by a defined policy before backend conversion.
- Backend output: internal to KotoAudio/HAL. It may be PWM compare values, I2S PCM, DAC samples, or another hardware-specific format.
- Normal apps: cannot observe or choose backend format.

This keeps codec choice independent from backend choice. A clip encoded as SLDPCM4 and a clip stored as PCM16 both enter the same mixer contract after decode.

## 9. Asset Converter Validation

The asset converter is the enforcement point for codec policy.

Input validation:

- WAV must be readable and have a known sample format.
- Channel count must be mono for v0 output. Stereo input may be rejected or downmixed on PC according to converter option.
- Sample rate must be converted on PC to the configured KotoAudio mixer rate.
- Duration must fit clip policy and target memory budget.
- Loop points must be inside decoded sample range and aligned to sample boundaries.

Codec validation:

- PCM16 is always available as fallback if the clip fits budget.
- PCM8 requires noise and peak-error checks against PCM16 reference.
- ADPCM4 requires decode-state and loop checkpoint validation if enabled.
- SLDPCM4 requires decode-then-compare validation using the exact runtime table and selected `step_shift`.

SLDPCM4-specific checks:

- `step_shift` within supported range.
- No unexpected payload overrun when decoding `sample_count`.
- Padding nibble ignored safely for odd sample counts.
- Saturation count reported.
- Peak absolute error reported.
- RMS error reported.
- Optional SNR or segmental SNR reported.
- Loop transition error measured at `loop_end -> loop_start`.
- Worst-case decode cost estimate included in asset report.

Converter output should include a human-readable report for debug builds or asset pipeline logs. A recommended report includes codec, source WAV parameters, output sample rate, decoded sample count, encoded bytes, compression ratio, estimated decode operations, peak error, RMS error, saturation count, and loop validation result.

## 10. v0/v1 Adoption Policy

### v0

v0 must ship with PCM16 clip support. This is the correctness baseline and the golden path for tests.

Recommended v0 policy:

- Required: PCM16 mono at mixer sample rate.
- Optional: PCM8 if the mixer path already handles PCM16 expansion cleanly and converter quality gates are in place.
- Experimental: SLDPCM4 behind the `experimental-sldpcm4` runtime build flag or an asset-pipeline flag, only for curated SFX. Builds without the feature parse the reserved KACL codec id but reject assets as unsupported.
- Deferred: ADPCM4 unless there is a clear capacity problem that PCM16/PCM8 cannot solve.

SLDPCM4 should not be the only supported compressed codec in v0, and should not be required for normal app compatibility. A v0 app should work when all clips are emitted as PCM16, assuming memory budget is met.

### v1

v1 may promote one 4bit clip codec after measurement.

Promotion criteria:

- Decode time remains within mixer block budget under maximum active source count.
- Audio quality is acceptable for common SFX categories.
- Loop clicks and predictor discontinuities are controlled by converter validation.
- Asset reports make codec failures understandable.
- Runtime implementation remains smaller and simpler than storing PCM16 for the target content set.

If SLDPCM4 performs well for short percussive/UI sounds but poorly for tonal clips, v1 may keep it as a profile-specific codec and use PCM16 or ADPCM4 for other content.

If ADPCM4 outperforms SLDPCM4 on quality per byte with acceptable decoder cost, ADPCM4 should be preferred for general compressed clips. SLDPCM4 can remain as a simple-fast option or be dropped before ABI freeze.

## 11. Risks and Measurement Items

Risks:

- SLDPCM4 fixed table may produce audible zipper noise, especially near silence.
- Large transients may saturate or lag because the maximum delta is table-limited.
- `step_shift` that handles loud transients may make quiet tails noisy.
- Loop predictor mistakes can create clicks even when loop sample positions are correct.
- Multiple compressed sources may fit flash but exceed mixer decode budget.
- Codec proliferation can leak through app APIs if asset boundaries are not enforced.
- A premature SLDPCM4 ABI could block better ADPCM4 or PCM-only choices later.

Measurement items:

- Decode cycles per sample for PCM16, PCM8, ADPCM4, and SLDPCM4.
- Max mix time per block at target active source counts.
- Flash/RAM size per representative SFX set.
- Peak error, RMS error, saturation count, and subjective listening notes.
- Loop transition discontinuity for looped clips.
- Failure/fallback rate across real asset corpus.
- Mixer underrun count with compressed and uncompressed mixes.
- Power impact of compressed decode versus larger PCM reads.
- Converter runtime and report clarity for content authors.

Initial decision rule:

- Use PCM16 until measurement proves a compressed codec is needed.
- Use compressed codecs only when they reduce a real budget pressure.
- Prefer asset converter fallback over runtime flexibility.
- Keep codec internals private to KotoAudio until v1 measurement stabilizes.
