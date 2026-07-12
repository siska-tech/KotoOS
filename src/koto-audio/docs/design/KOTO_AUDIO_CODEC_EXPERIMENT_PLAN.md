# KotoAudio Clip Codec Experiment Plan

## 0. Scope

この文書は、KotoAudio v0/v1でshort clip/SFX向けcodecを採用するかを判断するための測定計画である。入力文書は `docs/design/KOTO_AUDIO_REQUIREMENTS.md` と `docs/design/KOTO_AUDIO_CODEC_POLICY.md` であり、本計画はそれらの要求と方針を実装前に検証するためのものとして扱う。

KotoAudioはPicoCalc/RP2040向けのbounded audio runtimeである。codecは通常アプリへ露出する互換仕様ではなく、KotoAudio内部のasset/runtime contractである。P/ECE互換、既存SLDPCM bitstream互換、既存SLDPCM table互換は目指さない。

この文書では実装コードを書かない。codecの最終仕様も確定しすぎない。測定対象、測定方法、report、採否判断を定義し、実装詳細は後続の設計・実装タスクで扱う。

## 1. Experiment Goals

実験目的は、PCM16、PCM8、SLDPCM4、ADPCM4をKotoAudio v0/v1のclip codecとしてどの段階で採用するかを、容量、品質、loop安全性、runtime負荷、converter運用性から判断することである。

確認すること:

- PCM16だけでv0 SFX要件を満たせるか。
- PCM8がv0で安全に使える容量節約profileになるか。
- SLDPCM4がKotoAudio独自のsimple-fast 4bit codecとして有用か。
- SLDPCM4 decode tableの候補差が品質と失敗率にどれだけ影響するか。
- ADPCM4をv0で後回しにしてよいか、またはv1 stable候補として優先すべきか。
- codec別のloop discontinuity、saturation、underrun、max mix timeがbounded audio runtimeの予算内に収まるか。
- converter reportだけでcontent authorとruntime engineerが採否理由を追跡できるか。

## 2. Codec Candidates

### PCM16

PCM16 monoは基準形式であり、品質評価のreferenceである。v0 required候補として扱い、他codecのcompression ratio、error、SNR、loop discontinuityはPCM16 decoded referenceと比較する。

測定上の役割:

- golden output。
- fallback codec。
- mixer入力品質の基準。
- 圧縮codecが不要である可能性の確認。

### PCM8

PCM8 monoは低decode costで容量を半分にする候補である。符号、中心値、scaleの細部はこの計画では固定しない。実験では「PCM16 referenceへ復元した結果」を評価対象にする。

測定上の役割:

- v0 optional候補。
- UI音やnoise系SFXで十分かの確認。
- tonal sustain、short melody、looped ambienceで量子化ノイズが許容できるかの確認。

### SLDPCM4 current table

SLDPCM4 current tableは、`KOTO_AUDIO_CODEC_POLICY.md` にある現行候補tableを使う実験対象である。KotoAudio独自候補であり、既存SLDPCM互換とは扱わない。

測定上の役割:

- simple fixed table方式の基準。
- decode cycles/sampleを小さく保てるかの確認。
- `step_shift`探索とfallback policyの有効性確認。
- transient、quiet tail、loop pointでの失敗傾向の把握。

### SLDPCM4 original-like table

SLDPCM4 original-like tableは、既存SLDPCM風の差分分布を参考にした比較用tableである。ただし、KotoAudio assetとしてのbitstream、metadata、loop state、converter validationは独自扱いにする。互換decodeや互換asset生成は目的にしない。

測定上の役割:

- current tableとの差分比較。
- 小さい差分、急峻な差分、静音付近の表現力比較。
- KotoAudio用tableを固定する価値があるか、またはtable探索を続けるべきかの確認。

この候補は比較実験用であり、測定前に公開ABIやruntime必須tableとして扱わない。

### ADPCM4 Optional

ADPCM4は4bit/sampleの任意候補である。adaptive stepを持つ一般的な4bit圧縮の比較対象として扱うが、P/ECE互換ADPCMは目指さない。predictor、step index、block restart、loop checkpointなどはKotoAudio独自仕様として後続設計で決める。

測定上の役割:

- v1 stableの一般圧縮codec候補。
- SLDPCM4より品質/byteが良いかの比較。
- decode costとloop state複雑性がKotoAudioに見合うかの確認。

v0ではoptionalまたはdeferred候補とし、実験結果なしにrequiredへ昇格しない。

## 3. Representative SFX Corpus

実験corpusは、短いSFXとloop素材の両方を含む。各素材は可能なら複数variantを用意し、音量、帯域、attack、loop有無の違いを含める。素材はKotoAudio独自の評価用assetとして管理し、P/ECE素材をコピーしない。

| Category | Purpose | Notes |
|---|---|---|
| UI click | 短いtransientと静音復帰 | 小音量版、大音量版を含める |
| cursor move | 連続再生されやすい短音 | 連打時のmix負荷と耳障りなnoiseを確認する |
| confirm/cancel | ゲームUIで目立つ短いtonal SFX | PCM8/SLDPCM4のpitch感と余韻劣化を見る |
| explosion/noise | noise、transient、saturation耐性 | 4bit codecが得意な可能性と飽和を確認する |
| short melody | 短い音程列 | pitch誤差、quantization noise、tailを確認する |
| tonal sustain | 持続音、減衰音 | SNR、静音付近のzipper noise、loop候補を確認する |
| looped ambience | 短いloop環境音 | loop discontinuity、長時間再生、underrunを確認する |

最小corpus:

- 各category 3素材以上。
- 可能なら16kHz用と22.05kHz用の評価出力を作る。
- looped ambienceとtonal sustainはloop metadata付きvariantを含める。
- すべての素材にPCM16 referenceを保持する。

## 4. Metrics

測定値はconverter reportとruntime/host benchmark reportに分けて記録する。codec候補ごと、素材ごと、sample rate候補ごとに同じ項目を出す。

### Size and Compression

| Metric | Meaning |
|---|---|
| encoded size | codec payload、必要metadata、loop checkpointを含むasset bytes |
| compression ratio | PCM16 reference size / encoded size |

encoded sizeはpayloadだけでなく、runtime-ready assetとして必要なheader、codec metadata、loop state、restart/checkpointがある場合はそれも含める。

### Objective Quality

| Metric | Meaning |
|---|---|
| peak error | `max(abs(reference - decoded))` |
| RMS error | referenceとdecodedのroot mean square error |
| SNR | reference signal powerとerror powerから算出するdB値 |
| saturation count | encodeまたはdecode過程でsigned PCM16範囲へsaturateした回数 |
| loop discontinuity | loop endからloop startへ戻る境界のsample差、短窓RMS差、またはclick proxy |

SNRは無音に近い素材で不安定になるため、reference RMSが小さい場合はSNRを参考値として扱い、peak error、RMS error、listening noteを優先する。

loop discontinuityは少なくとも以下を分けて記録する:

- boundary absolute jump。
- loop前後の短窓RMS差。
- predictor/state復元が必要なcodecでのstate mismatch有無。

### Runtime Budget

| Metric | Meaning |
|---|---|
| decode cycles/sample | codec decoderだけの平均、p95、worst observed |
| max mix time/block | target source数で1blockをmixする最大時間 |
| underrun count | backendまたはmock backendでsubmitが間に合わなかった回数 |

runtime測定は、単独sourceだけでなくv0/v1想定の同時再生で行う。

推奨測定case:

- 1 active SFX。
- 4 active SFX。
- 4 active SFX + 1 BGM placeholderまたはsequence-like load。
- 同一codecのみ。
- PCM16と圧縮codecの混在。
- looped ambienceを含む長時間run。

## 5. Subjective Listening Checklist

主観評価は数値metricの代替ではなく、採否判断のgateとして使う。評価者はreference PCM16とcodec decoded outputを音量差に注意して比較する。

Checklist:

- Attackが鈍っていないか。
- UI clickやcursor moveで耳障りなtick、buzz、zipper noiseがないか。
- confirm/cancelで音程感や明るさが崩れていないか。
- explosion/noiseで破綻が目立たず、飽和が不自然でないか。
- short melodyでpitch感、音の終端、音間のノイズが許容できるか。
- tonal sustainで静音付近の粒状感、揺れ、歪みが目立たないか。
- looped ambienceでloop click、周期的な揺れ、長時間再生時の疲労感がないか。
- 複数source同時再生時にcodec由来のノイズが積み上がっていないか。
- 小音量再生時とmaster volume変更時に劣化が目立たないか。

主観評価は `pass`、`borderline`、`fail` と短いnoteで記録する。`borderline` は用途限定採用の候補にできるが、v0 requiredやv1 stableの根拠には単独で使わない。

## 6. Converter Report Format

converterは素材ごと、codec候補ごとに人間可読reportを出す。machine-readable reportも同じ内容を持つことが望ましいが、この計画では形式を固定しない。

Required fields:

| Field | Description |
|---|---|
| asset id | 入力素材を追跡するID |
| source file | 元WAVまたは生成元 |
| source parameters | sample rate、channels、bit depth、duration、loop metadata |
| output parameters | KotoAudio sample rate、mono/stereo、decoded sample count |
| codec candidate | PCM16、PCM8、SLDPCM4 current table、SLDPCM4 original-like table、ADPCM4 optional |
| codec parameters | table id、step shift、block/restart設定など。未確定値はexperimentalとして記録する |
| encoded size | runtime-ready asset bytes |
| compression ratio | PCM16 reference比 |
| peak error | objective quality metric |
| RMS error | objective quality metric |
| SNR | dB、またはlow-signal参考値 |
| saturation count | encode/decode saturation回数 |
| loop discontinuity | loop境界metricとstate validation結果 |
| estimated decode cycles/sample | host推定または実機測定値 |
| estimated max mix time/block | 対象scenarioとともに記録 |
| underrun count | 実行測定時のみ。未測定ならN/A |
| converter decision | accept、fallback、reject、needs listening |
| fallback codec | 不採用時のfallback |
| warnings | 品質、loop、metadata、budget上の警告 |

Example shape:

```text
asset: ui_click_01
codec: SLDPCM4 current table
decision: fallback
fallback: PCM16
encoded_size_bytes: 184
compression_ratio_vs_pcm16: 3.72
peak_error: 9200
rms_error: 1180.4
snr_db: 18.6
saturation_count: 14
loop_discontinuity: N/A
decode_cycles_per_sample: N/A
max_mix_time_per_block: N/A
warnings:
  - transient peak error exceeds v0 threshold
```

## 7. Acceptance Thresholds

閾値は初期値であり、実機測定後に更新できる。更新する場合は、同じcorpusで再測定し、変更理由をreportに残す。

### v0 Required Thresholds

PCM16:

- encoded sizeがv0 memory/flash budget内に収まること。
- decoded outputがreferenceと同一、またはconverter処理後referenceとして扱えること。
- decode cycles/sampleがmixer budget上ほぼ無視できること。
- 4 active SFXでmax mix time/blockがblock deadlineの50%以下を目標にすること。
- underrun countが通常負荷testで0であること。

### v0 Optional Thresholds

PCM8:

- compression ratioがPCM16比でおおむね1.8以上であること。
- UI click、cursor move、explosion/noiseの主観評価がpassまたは用途限定borderlineであること。
- peak errorとRMS errorが素材category別の許容範囲に収まること。
- looped ambienceまたはtonal sustainでfailが多い場合、用途限定profileに留めること。
- decode cycles/sampleがPCM16に近く、max mix time/blockを悪化させないこと。

SLDPCM4:

- compression ratioがPCM16比でおおむね3.0以上であること。
- 対象categoryで主観評価passが多数で、fail素材はconverterがPCM16またはPCM8へfallbackできること。
- saturation countが少なく、saturationが聴感上の破綻につながらないこと。
- loop discontinuityがPCM系より明確に悪化する場合、loop素材では不採用にすること。
- decode cycles/sampleとmax mix time/blockがv0 source数でblock deadline内に収まること。

### v1 Stable Thresholds

v1 stableへ昇格するcodecは、以下を満たす必要がある。

- representative corpus全体でfallback率が許容範囲に収まること。
- categoryごとの不向きがreportで明確で、converterが自動または明示的に回避できること。
- looped ambienceとtonal sustainでloop artifactを制御できること。
- 4 active SFX + BGM相当負荷でmax mix time/blockがblock deadlineの70%以下を目標にすること。
- 長時間loop testでunderrun countが0であること。
- decode state、loop state、metadata validationがruntime APIへ漏れないこと。
- asset reportが採否理由を説明できること。

### Reject or Defer Thresholds

以下に該当するcodec候補はrejectまたはdeferする。

- PCM16 fallbackより容量以外の価値が乏しい。
- 主観評価failが多く、converterで安全にfallbackできない。
- loop discontinuityが目立ち、loop素材で回避策が複雑すぎる。
- decode cycles/sampleまたはmax mix time/blockがbounded runtime予算を圧迫する。
- saturation countが多く、clipごとの調整で安定しない。
- codec metadataやstateが通常アプリAPIへ漏れそうになる。
- 仕様固定前にtable、packing、block restartの未確定要素が多すぎる。

## 8. Decision Rules

### v0 Required

v0 requiredにできるのは、全v0 appが依存してよいcodecだけである。

Decision rule:

- PCM16 mono at mixer sample rateをv0 requiredにする。
- v0 required codecはconverter fallbackなしで常にdecode可能でなければならない。
- v0 required codecはnormal app互換性の基準になるため、実験codecを含めない。

### v0 Experimental

v0 experimentalは、build flag、asset-pipeline flag、またはdebug/curated asset用途に限定する。

Decision rule:

- PCM8は品質gateとreportが十分ならv0 optionalまたはexperimentalにできる。
- SLDPCM4 current tableとSLDPCM4 original-like tableは、測定完了までv0 experimentalに留める。
- ADPCM4 optionalは、容量問題が明確でない限りv0ではdeferまたはexperimental比較に留める。
- experimental codecで失敗したassetはPCM16へfallbackできること。

### v1 Stable

v1 stableは、KotoAudioの通常asset pipelineで使ってよいcodecである。ただし、通常アプリがcodec詳細を選ぶ設計にはしない。

Decision rule:

- 4bit codecをv1 stableへ昇格する場合、SLDPCM4またはADPCM4のどちらか一方を優先する。
- SLDPCM4がUI/noise系だけに強い場合、profile-specific stableとして扱い、tonal/loop素材はPCM16または別codecへfallbackする。
- ADPCM4が品質/byteで優れ、decode costとloop stateが許容範囲なら、general compressed clip候補として優先する。
- v1 stable codecはconverter report、loop validation、runtime counterで問題追跡できること。

### Reject/Defer

Decision rule:

- tableやmetadataが安定しないcodecはdeferする。
- P/ECE互換や既存SLDPCM互換を理由に採用しない。
- compressed codecが実際のbudget pressureを解決しない場合、PCM16/PCM8中心に戻す。
- runtimeを複雑化させる割にasset reportで採否が説明できないcodecはrejectする。

## 9. Risks

- SLDPCM4 fixed tableが静音付近でzipper noiseを出す。
- SLDPCM4の`step_shift`が大きいtransientと小さいtailの両方を満たせない。
- original-like table比較が互換性の誤解を生む。
- ADPCM4が品質面で有利でも、predictor、step index、restart、loop checkpointでruntimeとasset formatが複雑になる。
- PCM8がUI/noiseでは十分でも、melodyやsustainで耳障りになる。
- 圧縮でflashは減るが、decode負荷によりmax mix time/blockやunderrunが悪化する。
- loop stateのvalidation不足により、短いloop素材でclickが出る。
- subjective listeningの音量合わせが不十分だと、codec差を誤判定する。
- codec候補が増えすぎると、converter report、test matrix、runtime decoderが肥大化する。
- 早期にcodec ABIを固定すると、v1でより良いtableやADPCM4設計へ移りにくくなる。

## 10. Implementation-Free Measurement Roadmap

このroadmapは実装コードではなく、測定作業の順序を定義する。

### Phase 1: Corpus and Reference Preparation

- representative SFX corpusを集める。
- すべての素材をKotoAudio評価用PCM16 mono referenceへ正規化する。
- sample rate候補を決め、16kHzと22.05kHzなどの比較referenceを作る。
- loop素材にはloop_start、loop_end、期待する聴感をmetadataとして付ける。

Exit criteria:

- 各categoryに3素材以上ある。
- PCM16 referenceとloop metadataが追跡可能である。
- P/ECE素材や互換assetを使っていない。

### Phase 2: Offline Codec Evaluation

- PCM16、PCM8、SLDPCM4 current table、SLDPCM4 original-like table、ADPCM4 optionalを同じcorpusへ適用する。
- encoded size、compression ratio、peak error、RMS error、SNR、saturation countを記録する。
- loop素材ではloop discontinuityを記録する。
- converter decisionとfallback codecを仮記録する。

Exit criteria:

- codec候補ごとのobjective metricが揃っている。
- table候補の差がcategory別に比較できる。
- 明らかなreject/defer候補が分かる。

### Phase 3: Subjective Listening

- PCM16 referenceとcodec decoded outputを音量合わせして比較する。
- checklistに沿ってpass、borderline、fail、noteを記録する。
- 数値metricと聴感がずれる素材を抽出する。
- 用途限定profileが成立するcategoryを確認する。

Exit criteria:

- 各codec候補の得意/不得意categoryが説明できる。
- converterで自動fallbackすべき条件の候補がある。
- v0 experimentalに入れてよい候補と避ける候補が分かる。

### Phase 4: Runtime Budget Measurement

- codec decoder単体のdecode cycles/sampleを測る。
- target block sizeとsource数でmax mix time/blockを測る。
- 1 active SFX、4 active SFX、4 active SFX + BGM相当負荷を測る。
- looped ambienceを含む長時間runでunderrun countを測る。

Exit criteria:

- PCM16 v0 requiredのruntime余裕が確認できる。
- PCM8や4bit codecのdecode負荷がblock deadline内か判断できる。
- 圧縮codecがflash削減と引き換えにunderrun riskを増やすか判断できる。

### Phase 5: Adoption Decision

- acceptance thresholdsに照らして、各codecをv0 required、v0 experimental、v1 stable候補、reject/deferへ分類する。
- 採用するcodecごとに、未確定仕様を後続設計項目として分離する。
- reject/defer理由をreportに残す。
- KotoAudio codec policyへ反映する変更点を整理する。

Exit criteria:

- v0がPCM16だけで成立するか明確である。
- v0 experimental codecの範囲が限定されている。
- v1 stableに向けて測定を継続するcodecが絞られている。
- KotoAudio独自仕様として扱う境界が維持されている。

## 11. Expected Output

この測定計画の成果物は、codec実装そのものではなく、採否判断に必要な記録である。

Expected outputs:

- corpus manifest。
- per-asset converter reports。
- codec comparison summary。
- subjective listening notes。
- runtime budget measurement report。
- v0/v1 adoption decision note。
- follow-up design tasks for any codec promoted beyond experimental.

最終判断では、PCM16を基準にし、圧縮codecは「容量削減が実際のbudget pressureを解決し、音質とruntime負荷が許容できる場合だけ」採用する。
