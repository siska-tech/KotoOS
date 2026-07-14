//! Host-side asset conversion tools for KotoAudio.
//!
//! This crate intentionally uses `std` and stays outside the `no_std`
//! runtime crate. v0 conversion accepts common PCM16 WAV inputs, performs
//! host-side downmix/resampling when needed, and emits runtime-ready clip
//! assets.

pub mod mml;

use koto_audio::{
    parse_clip_asset, validate_compact_sequence, AssetPlacement, AudioBackend, AudioError,
    AudioEventKind, AudioLimits, AudioPolicy, AudioService, BackendError, BackendReport,
    BackendResult, BackendState, ClipAsset, ClipAssetError, ClipAssetHeader, ClipLoop, CodecId,
    CompactEvent, CompactInstrument, CompactSequence, CompactSequenceError, CompactTempo,
    CompactTrack, MixerBlock, MixerVolume, PolyphonicSequence, PolyphonicSequenceVoice, Sequence,
    SequenceEvent, PCM16_MONO_CHANNELS, SEQUENCE_REPEAT_INFINITE,
};
use std::{cell::RefCell, rc::Rc};

/// Conversion options for WAV to runtime-ready clip assets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvertOptions {
    /// Runtime limits used for sample-rate validation.
    pub limits: AudioLimits,
    /// Loop metadata to write into the asset.
    pub loop_metadata: ClipLoop,
    /// Placement hint to write into the asset.
    pub placement: AssetPlacement,
    /// Optional memory budget hint in bytes. Zero means unspecified.
    pub budget_hint_bytes: u32,
    /// Requested output codec. Defaults to the v0 required PCM16 path.
    pub output_codec: OutputCodec,
    /// Target runtime sample rate for the emitted asset.
    pub target_sample_rate_hz: u32,
    /// Reject non-mono or mismatched sample-rate input instead of converting it.
    pub strict_input: bool,
    /// Optional cap applied after downmix/resampling. Useful for bounded one-shot assets.
    pub max_output_samples: Option<usize>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        let limits = AudioLimits::v0_default();
        Self {
            limits,
            loop_metadata: ClipLoop::None,
            placement: AssetPlacement::Unspecified,
            budget_hint_bytes: 0,
            output_codec: OutputCodec::Pcm16,
            target_sample_rate_hz: limits.sample_rate_hz,
            strict_input: false,
            max_output_samples: None,
        }
    }
}

/// Converter output codec selection.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OutputCodec {
    /// Required v0 PCM16 output.
    #[default]
    Pcm16,
    /// Experimental SLDPCM4 output. This is never selected by default.
    ExperimentalSldpcm4 {
        /// How the converter should behave when SLDPCM4 is unsuitable.
        fallback: Sldpcm4FallbackPolicy,
    },
}

/// Experimental SLDPCM4 fallback behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Sldpcm4FallbackPolicy {
    /// Fall back to PCM16 when experimental SLDPCM4 is unsuitable.
    Pcm16,
    /// Reject conversion instead of falling back.
    Reject,
    /// Write SLDPCM4 when quality is questionable, while still rejecting invalid loops.
    ForceExperimental,
}

impl Default for Sldpcm4FallbackPolicy {
    fn default() -> Self {
        Self::Pcm16
    }
}

/// Converter output decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConverterDecision {
    /// The requested codec was accepted.
    Accept,
    /// The converter wrote a fallback codec.
    Fallback,
    /// The converter rejected the input.
    Reject,
    /// The output is experimental and should be evaluated by listening.
    NeedsListening,
}

/// Human-readable conversion report.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversionReport {
    /// Source file path or label.
    pub source_file: String,
    /// Source WAV sample rate.
    pub source_sample_rate_hz: u32,
    /// Source WAV channel count.
    pub source_channels: u16,
    /// Source WAV bits per sample.
    pub source_bit_depth: u16,
    /// Output asset sample rate.
    pub output_sample_rate_hz: u32,
    /// Target sample rate requested for conversion.
    pub target_sample_rate_hz: u32,
    /// Original WAV frame count before downmix or resampling.
    pub original_frame_count: u32,
    /// Output mono sample count after host-side conversion.
    pub output_sample_count: u32,
    /// Whether host-side downmixing was applied.
    pub downmix_applied: bool,
    /// Whether host-side resampling was applied.
    pub resample_applied: bool,
    /// Resampler used for host-side sample-rate conversion.
    pub resampler: &'static str,
    /// Decoded mono sample count.
    pub decoded_sample_count: u32,
    /// Output codec name.
    pub codec: &'static str,
    /// Whether the selected/requested codec is experimental.
    pub experimental: bool,
    /// Payload byte count.
    pub payload_bytes: u32,
    /// Encoded payload byte count for the selected output.
    pub encoded_payload_bytes: u32,
    /// Total asset byte count.
    pub total_asset_bytes: u32,
    /// Compression ratio versus PCM16 payload bytes.
    pub compression_ratio_vs_pcm16: f32,
    /// Peak absolute error versus the PCM16 reference.
    pub peak_absolute_error: Option<u16>,
    /// RMS error versus the PCM16 reference.
    pub rms_error: Option<f32>,
    /// Signal-to-noise ratio in dB.
    pub snr_db: Option<f32>,
    /// True when SNR is reported for very low-signal input.
    pub low_signal_snr_reference: bool,
    /// Count of saturating reconstructions during experimental encode/decode.
    pub saturation_count: Option<u32>,
    /// Loop metadata written to the asset.
    pub loop_metadata: ClipLoop,
    /// Loop validation result for the selected/requested codec.
    pub loop_validation_result: String,
    /// Converter decision.
    pub converter_decision: ConverterDecision,
    /// Fallback codec if a fallback was written.
    pub fallback_codec: Option<&'static str>,
    /// Experimental SLDPCM4 table identifier.
    pub sldpcm4_table_id: Option<&'static str>,
    /// Runtime validation result.
    pub validation_result: Result<(), ClipAssetError>,
    /// Human-readable warnings.
    pub warnings: Vec<String>,
}

impl ConversionReport {
    /// Formats the report for terminal output.
    pub fn to_human_readable(&self) -> String {
        let validation = match self.validation_result {
            Ok(()) => "ok".to_string(),
            Err(error) => format!("error: {error:?}"),
        };
        let warnings = if self.warnings.is_empty() {
            "none".to_string()
        } else {
            self.warnings.join("; ")
        };
        let peak = self
            .peak_absolute_error
            .map_or_else(|| "n/a".to_string(), |value| value.to_string());
        let rms = self
            .rms_error
            .map_or_else(|| "n/a".to_string(), |value| format!("{value:.3}"));
        let snr = self
            .snr_db
            .map_or_else(|| "n/a".to_string(), |value| format!("{value:.3} dB"));
        let saturation = self
            .saturation_count
            .map_or_else(|| "n/a".to_string(), |value| value.to_string());
        let fallback = self.fallback_codec.unwrap_or("none");
        let table_id = self.sldpcm4_table_id.unwrap_or("none");

        format!(
            concat!(
                "KotoAudio clip conversion report\n",
                "source file: {}\n",
                "source sample rate: {} Hz\n",
                "source channels: {}\n",
                "source bit depth: {}\n",
                "output sample rate: {} Hz\n",
                "target sample rate: {} Hz\n",
                "original frame count: {}\n",
                "output sample count: {}\n",
                "downmix applied: {}\n",
                "resample applied: {}\n",
                "resampler: {}\n",
                "decoded sample count: {}\n",
                "codec: {}\n",
                "SLDPCM4 table id: {}\n",
                "experimental codec: {}\n",
                "payload bytes: {}\n",
                "encoded payload bytes: {}\n",
                "total asset bytes: {}\n",
                "compression ratio vs PCM16: {:.3}\n",
                "peak absolute error: {}\n",
                "RMS error: {}\n",
                "SNR: {}\n",
                "SNR low-signal reference: {}\n",
                "saturation count: {}\n",
                "loop metadata: {:?}\n",
                "loop validation result: {}\n",
                "converter decision: {:?}\n",
                "fallback codec: {}\n",
                "validation result: {}\n",
                "warnings: {}\n"
            ),
            self.source_file,
            self.source_sample_rate_hz,
            self.source_channels,
            self.source_bit_depth,
            self.output_sample_rate_hz,
            self.target_sample_rate_hz,
            self.original_frame_count,
            self.output_sample_count,
            self.downmix_applied,
            self.resample_applied,
            self.resampler,
            self.decoded_sample_count,
            self.codec,
            table_id,
            self.experimental,
            self.payload_bytes,
            self.encoded_payload_bytes,
            self.total_asset_bytes,
            self.compression_ratio_vs_pcm16,
            peak,
            rms,
            snr,
            self.low_signal_snr_reference,
            saturation,
            self.loop_metadata,
            self.loop_validation_result,
            self.converter_decision,
            fallback,
            validation,
            warnings
        )
    }
}

/// Successful conversion output.
#[derive(Clone, Debug, PartialEq)]
pub struct ConversionOutput {
    /// Runtime-ready asset bytes.
    pub asset_bytes: Vec<u8>,
    /// Human-readable conversion report data.
    pub report: ConversionReport,
}

/// Human-readable KACL decode report.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodeReport {
    /// Source file path or label.
    pub source_file: String,
    /// Source KACL codec name.
    pub codec: &'static str,
    /// Whether the decoded codec is experimental.
    pub experimental: bool,
    /// Source KACL sample rate.
    pub sample_rate_hz: u32,
    /// Source KACL channel count.
    pub channels: u16,
    /// Decoded mono sample count.
    pub sample_count: u32,
    /// Encoded KACL payload byte count.
    pub payload_bytes: u32,
    /// Output WAV byte count.
    pub wav_bytes: u32,
    /// Output WAV format.
    pub output_format: &'static str,
    /// Experimental SLDPCM4 table identifier.
    pub sldpcm4_table_id: Option<&'static str>,
    /// Runtime validation result.
    pub validation_result: Result<(), ClipAssetError>,
}

impl DecodeReport {
    /// Formats the report for terminal output.
    pub fn to_human_readable(&self) -> String {
        let validation = match self.validation_result {
            Ok(()) => "ok".to_string(),
            Err(error) => format!("error: {error:?}"),
        };
        let table_id = self.sldpcm4_table_id.unwrap_or("none");

        format!(
            concat!(
                "KotoAudio clip decode report\n",
                "source file: {}\n",
                "codec: {}\n",
                "experimental codec: {}\n",
                "sample rate: {} Hz\n",
                "channels: {}\n",
                "sample count: {}\n",
                "payload bytes: {}\n",
                "output WAV bytes: {}\n",
                "output format: {}\n",
                "SLDPCM4 table id: {}\n",
                "validation result: {}\n"
            ),
            self.source_file,
            self.codec,
            self.experimental,
            self.sample_rate_hz,
            self.channels,
            self.sample_count,
            self.payload_bytes,
            self.wav_bytes,
            self.output_format,
            table_id,
            validation
        )
    }
}

/// Successful KACL decode output.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodeOutput {
    /// PCM16 mono WAV bytes.
    pub wav_bytes: Vec<u8>,
    /// Human-readable decode report data.
    pub report: DecodeReport,
}

/// Options for generating built-in drum Rust table fragments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrumTableOptions {
    /// Static symbol name to emit, such as `DRUM_BD`.
    pub symbol_name: String,
    /// Target runtime sample rate for the emitted PCM16 mono table.
    pub target_sample_rate_hz: u32,
    /// Reject non-mono or mismatched sample-rate input instead of converting it.
    pub strict_input: bool,
}

impl Default for DrumTableOptions {
    fn default() -> Self {
        Self {
            symbol_name: "DRUM_SAMPLE".to_string(),
            target_sample_rate_hz: AudioLimits::v0_default().sample_rate_hz,
            strict_input: false,
        }
    }
}

/// Generated built-in drum table fragment and source metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrumTableOutput {
    /// Rust source fragment containing a `pub static` PCM16 mono slice.
    pub rust_fragment: String,
    /// Output PCM16 mono sample count.
    pub sample_count: u32,
    /// Target sample rate used for the emitted table.
    pub sample_rate_hz: u32,
    /// Whether host-side downmixing was applied.
    pub downmix_applied: bool,
    /// Whether host-side resampling was applied.
    pub resample_applied: bool,
}

/// Host-side compact sequence table used by the generated Rust formatter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactSequenceTable {
    /// Compact instrument table to emit.
    pub instruments: Vec<CompactInstrument>,
    /// Monophonic track tables to emit.
    pub tracks: Vec<CompactSequenceTableTrack>,
    /// Runtime-ready compact tempo metadata.
    pub tempo: CompactTempo,
}

/// Host-side compact sequence track table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactSequenceTableTrack {
    /// Compact events for one monophonic track.
    pub events: Vec<CompactEvent>,
    /// Track-local fixed-point gain, where 256 is unity.
    pub gain: MixerVolume,
    /// Initial compact instrument table index.
    pub initial_instrument_id: u8,
}

/// Options for formatting compact sequence generated Rust tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactSequenceTableOptions {
    /// Final `CompactSequence` static symbol name.
    pub sequence_symbol_name: String,
    /// Prefix used for generated instrument, event, and track table symbols.
    pub table_symbol_prefix: String,
}

impl Default for CompactSequenceTableOptions {
    fn default() -> Self {
        Self {
            sequence_symbol_name: "COMPACT_SEQUENCE".to_string(),
            table_symbol_prefix: "COMPACT_SEQUENCE".to_string(),
        }
    }
}

/// Generated compact sequence Rust source fragment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompactSequenceTableOutput {
    /// Rust source fragment containing compact sequence static tables.
    pub rust_fragment: String,
    /// Instrument count emitted into the fragment.
    pub instrument_count: usize,
    /// Track count emitted into the fragment.
    pub track_count: usize,
    /// Per-track event counts emitted into the fragment.
    pub event_counts: Vec<usize>,
}

/// Options for rendering an experimental MML subset input to host-side PCM16 WAV.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmlRenderOptions {
    /// Output mixer/sample rate. Supported values are 16000, 22050, and 44100 Hz.
    pub sample_rate_hz: u32,
    /// Optional fixed render length in seconds.
    pub seconds: Option<u32>,
    /// Render through the BGM sequence admission path instead of SFX.
    pub bgm: bool,
    /// Tail silence appended to finite sequences when no fixed length is requested.
    pub finite_tail_silence_ms: u32,
}

impl Default for MmlRenderOptions {
    fn default() -> Self {
        Self {
            sample_rate_hz: 22_050,
            seconds: None,
            bgm: false,
            finite_tail_silence_ms: 100,
        }
    }
}

/// Host-side MML render report.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmlRenderReport {
    /// Source file path or label.
    pub source_file: String,
    /// Output WAV sample rate.
    pub sample_rate_hz: u32,
    /// Output PCM16 mono sample count.
    pub sample_count: u32,
    /// Rendered sequence track count.
    pub track_count: usize,
    /// Rendered compact instrument count.
    pub instrument_count: usize,
    /// True when BGM admission was used.
    pub bgm: bool,
    /// True when rendering stopped at an explicit seconds limit.
    pub seconds_limited: bool,
    /// True when the input contains an infinite compact loop.
    pub infinite_loop: bool,
}

impl MmlRenderReport {
    /// Formats the report for terminal output.
    pub fn to_human_readable(&self) -> String {
        format!(
            concat!(
                "KotoAudio MML render report\n",
                "source file: {}\n",
                "sample rate: {} Hz\n",
                "sample count: {}\n",
                "output format: PCM16 mono WAV\n",
                "track count: {}\n",
                "instrument count: {}\n",
                "bus: {}\n",
                "seconds limited: {}\n",
                "infinite loop: {}\n"
            ),
            self.source_file,
            self.sample_rate_hz,
            self.sample_count,
            self.track_count,
            self.instrument_count,
            if self.bgm { "BGM" } else { "SFX" },
            self.seconds_limited,
            self.infinite_loop
        )
    }
}

/// Successful host-side MML render output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MmlRenderOutput {
    /// PCM16 mono WAV bytes.
    pub wav_bytes: Vec<u8>,
    /// Render metadata.
    pub report: MmlRenderReport,
}

/// Converter failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConvertError {
    /// WAV container or PCM data is malformed.
    InvalidWav(&'static str),
    /// WAV format is not supported by v0 conversion.
    UnsupportedWav(&'static str),
    /// Source sample rate does not match the configured mixer rate.
    SampleRateMismatch { source: u32, expected: u32 },
    /// Source channel count is not mono.
    NonMono { channels: u16 },
    /// Runtime asset validation rejected the converted asset.
    RuntimeValidation(ClipAssetError),
    /// Experimental codec output was requested but rejected by converter policy.
    ExperimentalCodecRejected(&'static str),
    /// Generated Rust symbol name is not a valid identifier.
    InvalidRustSymbol,
}

/// Compact sequence table formatting failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompactSequenceTableError {
    /// Generated Rust symbol name is not a valid identifier.
    InvalidRustSymbol,
    /// Runtime compact sequence validation rejected the generated table shape.
    RuntimeValidation(CompactSequenceError),
}

/// KACL decode failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// Runtime asset validation rejected the source KACL.
    RuntimeValidation(ClipAssetError),
    /// The KACL codec is not supported by this tools build.
    UnsupportedCodec,
    /// WAV output would exceed the RIFF/WAVE size limits.
    WavTooLarge,
}

/// Host-side MML render failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MmlRenderError {
    /// The selected output sample rate is not supported by this tool.
    UnsupportedSampleRate,
    /// The requested seconds value is invalid or too large.
    InvalidSeconds,
    /// The MML subset parser rejected the input.
    Parse(mml::MmlParseError),
    /// Runtime compact sequence validation rejected the generated table shape.
    RuntimeValidation(CompactSequenceError),
    /// Runtime sequence playback setup failed.
    Runtime(AudioError),
    /// An input with an infinite loop needs a finite render length.
    InfiniteLoopNeedsSeconds,
    /// WAV output would exceed the RIFF/WAVE size limits.
    WavTooLarge,
}

/// Converts a PCM16 WAV byte slice into a runtime-ready clip asset.
pub fn convert_wav_to_clip_asset(
    source_file: impl Into<String>,
    wav_bytes: &[u8],
    options: ConvertOptions,
) -> Result<ConversionOutput, ConvertError> {
    let source_file = source_file.into();
    let wav = parse_wav(wav_bytes)?;

    if options.target_sample_rate_hz == 0 {
        return Err(ConvertError::UnsupportedWav(
            "target sample rate must be non-zero",
        ));
    }
    if wav.bits_per_sample != 16 {
        return Err(ConvertError::UnsupportedWav(
            "only PCM16 WAV input is supported",
        ));
    }
    if options.strict_input && wav.sample_rate_hz != options.target_sample_rate_hz {
        return Err(ConvertError::SampleRateMismatch {
            source: wav.sample_rate_hz,
            expected: options.target_sample_rate_hz,
        });
    }
    if options.strict_input && wav.channels != u16::from(PCM16_MONO_CHANNELS) {
        return Err(ConvertError::NonMono {
            channels: wav.channels,
        });
    }

    let original_frame_count = wav_frame_count(wav)?;
    let downmix_applied = wav.channels != u16::from(PCM16_MONO_CHANNELS);
    let resample_applied = wav.sample_rate_hz != options.target_sample_rate_hz;
    let mono_samples = wav_mono_samples(wav)?;
    let mut reference_samples = if resample_applied {
        resample_linear(
            &mono_samples,
            wav.sample_rate_hz,
            options.target_sample_rate_hz,
        )?
    } else {
        mono_samples
    };
    if let Some(max_samples) = options.max_output_samples {
        reference_samples.truncate(max_samples);
    }
    let pcm16_payload = pcm16_payload_from_samples(&reference_samples);
    let sample_count = u32::try_from(reference_samples.len())
        .map_err(|_| ConvertError::UnsupportedWav("WAV is too large"))?;
    let pcm16_payload_bytes = u32::try_from(pcm16_payload.len())
        .map_err(|_| ConvertError::UnsupportedWav("WAV payload is too large"))?;

    let encoded = match options.output_codec {
        OutputCodec::Pcm16 => EncodedClip::pcm16(&pcm16_payload, sample_count),
        OutputCodec::ExperimentalSldpcm4 { fallback } => encode_experimental_sldpcm4_or_fallback(
            &pcm16_payload,
            &reference_samples,
            sample_count,
            options.loop_metadata,
            fallback,
        )?,
    };
    let validation_limits = AudioLimits {
        sample_rate_hz: options.target_sample_rate_hz,
        ..options.limits
    };

    let clip = ClipAsset {
        codec: encoded.codec_id,
        sample_rate_hz: options.target_sample_rate_hz,
        channels: PCM16_MONO_CHANNELS,
        sample_count,
        payload: &encoded.payload,
        loop_metadata: options.loop_metadata,
        placement: options.placement,
    };
    if encoded.codec_id == CodecId::Pcm16 {
        clip.validate_detailed(validation_limits)
            .map_err(ConvertError::RuntimeValidation)?;
    }

    let header = ClipAssetHeader::from_clip(clip, options.budget_hint_bytes)
        .map_err(ConvertError::RuntimeValidation)?;
    let mut asset_bytes =
        Vec::with_capacity(usize::from(header.header_size) + encoded.payload.len());
    asset_bytes.extend_from_slice(&header.encode());
    asset_bytes.extend_from_slice(&encoded.payload);

    let validation_result = if encoded.codec_id == CodecId::Pcm16 {
        parse_clip_asset(&asset_bytes, validation_limits).map(|_| ())
    } else {
        Ok(())
    };
    if let Err(error) = validation_result {
        return Err(ConvertError::RuntimeValidation(error));
    }

    let total_asset_bytes = u32::try_from(asset_bytes.len())
        .map_err(|_| ConvertError::UnsupportedWav("asset is too large"))?;
    let report = ConversionReport {
        source_file,
        source_sample_rate_hz: wav.sample_rate_hz,
        source_channels: wav.channels,
        source_bit_depth: wav.bits_per_sample,
        output_sample_rate_hz: options.target_sample_rate_hz,
        target_sample_rate_hz: options.target_sample_rate_hz,
        original_frame_count,
        output_sample_count: sample_count,
        downmix_applied,
        resample_applied,
        resampler: if resample_applied { "linear" } else { "none" },
        decoded_sample_count: sample_count,
        codec: encoded.codec_name,
        experimental: encoded.experimental,
        payload_bytes: encoded.payload_bytes,
        encoded_payload_bytes: encoded.payload_bytes,
        total_asset_bytes,
        compression_ratio_vs_pcm16: encoded.payload_bytes as f32
            / pcm16_payload_bytes.max(1) as f32,
        peak_absolute_error: encoded
            .metrics
            .as_ref()
            .map(|metrics| metrics.peak_absolute_error),
        rms_error: encoded.metrics.as_ref().map(|metrics| metrics.rms_error),
        snr_db: encoded.metrics.as_ref().map(|metrics| metrics.snr_db),
        low_signal_snr_reference: encoded
            .metrics
            .as_ref()
            .is_some_and(|metrics| metrics.low_signal_snr_reference),
        saturation_count: encoded
            .metrics
            .as_ref()
            .map(|metrics| metrics.saturation_count),
        loop_metadata: options.loop_metadata,
        loop_validation_result: encoded.loop_validation_result,
        converter_decision: encoded.decision,
        fallback_codec: encoded.fallback_codec,
        sldpcm4_table_id: encoded.sldpcm4_table_id,
        validation_result: Ok(()),
        warnings: encoded.warnings,
    };

    Ok(ConversionOutput {
        asset_bytes,
        report,
    })
}

/// Generates a Rust static table fragment from PCM16 WAV for built-in drums.
///
/// The output is signed PCM16 mono at `options.target_sample_rate_hz` and is
/// meant for future replacement of fixed sequence drum placeholder arrays. It
/// does not produce KACL and does not add WAV parsing to the runtime crate.
pub fn generate_drum_table_from_wav(
    wav_bytes: &[u8],
    options: DrumTableOptions,
) -> Result<DrumTableOutput, ConvertError> {
    if !valid_rust_static_symbol(&options.symbol_name) {
        return Err(ConvertError::InvalidRustSymbol);
    }
    if options.target_sample_rate_hz == 0 {
        return Err(ConvertError::UnsupportedWav(
            "target sample rate must be non-zero",
        ));
    }

    let wav = parse_wav(wav_bytes)?;
    if wav.bits_per_sample != 16 {
        return Err(ConvertError::UnsupportedWav(
            "only PCM16 WAV input is supported",
        ));
    }
    if options.strict_input && wav.sample_rate_hz != options.target_sample_rate_hz {
        return Err(ConvertError::SampleRateMismatch {
            source: wav.sample_rate_hz,
            expected: options.target_sample_rate_hz,
        });
    }
    if options.strict_input && wav.channels != u16::from(PCM16_MONO_CHANNELS) {
        return Err(ConvertError::NonMono {
            channels: wav.channels,
        });
    }

    let downmix_applied = wav.channels != u16::from(PCM16_MONO_CHANNELS);
    let resample_applied = wav.sample_rate_hz != options.target_sample_rate_hz;
    let mono_samples = wav_mono_samples(wav)?;
    let samples = if resample_applied {
        resample_linear(
            &mono_samples,
            wav.sample_rate_hz,
            options.target_sample_rate_hz,
        )?
    } else {
        mono_samples
    };
    let sample_count = u32::try_from(samples.len())
        .map_err(|_| ConvertError::UnsupportedWav("WAV is too large"))?;

    Ok(DrumTableOutput {
        rust_fragment: format_drum_table(
            &options.symbol_name,
            &samples,
            options.target_sample_rate_hz,
        ),
        sample_count,
        sample_rate_hz: options.target_sample_rate_hz,
        downmix_applied,
        resample_applied,
    })
}

/// Formats a validated compact sequence as Rust static table fragments.
///
/// This helper is intentionally not a KotoMML or `.kmml` parser. It accepts a
/// small host-side representation, validates it through the runtime compact
/// sequence boundary, and emits source suitable for firmware-owned or
/// game-owned static audio tables.
pub fn format_compact_sequence_table(
    table: &CompactSequenceTable,
    options: CompactSequenceTableOptions,
) -> Result<CompactSequenceTableOutput, CompactSequenceTableError> {
    if !valid_rust_static_symbol(&options.sequence_symbol_name)
        || !valid_rust_static_symbol(&options.table_symbol_prefix)
    {
        return Err(CompactSequenceTableError::InvalidRustSymbol);
    }

    let runtime_tracks: Vec<CompactTrack<'_>> = table
        .tracks
        .iter()
        .map(|track| CompactTrack::new(&track.events, track.gain, track.initial_instrument_id))
        .collect();
    validate_compact_sequence(CompactSequence::new(
        &table.instruments,
        &runtime_tracks,
        table.tempo,
    ))
    .map_err(CompactSequenceTableError::RuntimeValidation)?;

    Ok(CompactSequenceTableOutput {
        rust_fragment: format_compact_sequence_fragment(table, &options),
        instrument_count: table.instruments.len(),
        track_count: table.tracks.len(),
        event_counts: table
            .tracks
            .iter()
            .map(|track| track.events.len())
            .collect(),
    })
}

/// Decodes a runtime-ready KACL clip asset into PCM16 mono WAV bytes.
pub fn decode_clip_asset_to_wav(
    source_file: impl Into<String>,
    asset_bytes: &[u8],
) -> Result<DecodeOutput, DecodeError> {
    let source_file = source_file.into();
    let header = ClipAssetHeader::decode(asset_bytes).map_err(DecodeError::RuntimeValidation)?;
    let limits = AudioLimits {
        sample_rate_hz: header.sample_rate_hz,
        ..AudioLimits::v0_default()
    };
    let clip = parse_clip_asset(asset_bytes, limits).map_err(DecodeError::RuntimeValidation)?;

    let (samples, codec_name, experimental, table_id) = match clip.codec {
        CodecId::Pcm16 => (decode_pcm16_payload(clip.payload), "PCM16", false, None),
        CodecId::Sldpcm4 => {
            #[cfg(feature = "experimental-sldpcm4")]
            {
                (
                    decode_sldpcm4_payload(clip.payload, clip.sample_count),
                    "SLDPCM4",
                    true,
                    Some("standard16"),
                )
            }
            #[cfg(not(feature = "experimental-sldpcm4"))]
            {
                return Err(DecodeError::UnsupportedCodec);
            }
        }
        CodecId::Unsupported(_) => return Err(DecodeError::UnsupportedCodec),
    };

    let wav_bytes = write_pcm16_mono_wav(&samples, clip.sample_rate_hz)?;
    let wav_byte_count = u32::try_from(wav_bytes.len()).map_err(|_| DecodeError::WavTooLarge)?;
    let payload_bytes = u32::try_from(clip.payload.len()).map_err(|_| DecodeError::WavTooLarge)?;

    Ok(DecodeOutput {
        wav_bytes,
        report: DecodeReport {
            source_file,
            codec: codec_name,
            experimental,
            sample_rate_hz: clip.sample_rate_hz,
            channels: u16::from(clip.channels),
            sample_count: clip.sample_count,
            payload_bytes,
            wav_bytes: wav_byte_count,
            output_format: "PCM16 mono WAV",
            sldpcm4_table_id: table_id,
            validation_result: Ok(()),
        },
    })
}

/// Parses the experimental MML subset and renders it to host-side PCM16 mono WAV.
///
/// This helper intentionally lives in the tools crate. It uses the runtime
/// compact sequence validation and `AudioService` sequence admission path, but
/// does not add WAV writing, MML parsing, heap use, or new dependencies to the
/// runtime crate.
pub fn render_mml_to_wav(
    source_file: impl Into<String>,
    source: &str,
    options: MmlRenderOptions,
) -> Result<MmlRenderOutput, MmlRenderError> {
    validate_render_options(&options)?;
    let source_file = source_file.into();
    let table = mml::parse_mml_to_compact_sequence_table(source).map_err(MmlRenderError::Parse)?;
    let samples = render_compact_sequence_table(&table, &options)?;
    let sample_count = u32::try_from(samples.len()).map_err(|_| MmlRenderError::WavTooLarge)?;
    let wav_bytes = write_pcm16_mono_wav(&samples, options.sample_rate_hz)
        .map_err(|_| MmlRenderError::WavTooLarge)?;
    let infinite_loop = compact_table_has_infinite_loop(&table);

    Ok(MmlRenderOutput {
        wav_bytes,
        report: MmlRenderReport {
            source_file,
            sample_rate_hz: options.sample_rate_hz,
            sample_count,
            track_count: table.tracks.len(),
            instrument_count: table.instruments.len(),
            bgm: options.bgm,
            seconds_limited: options.seconds.is_some(),
            infinite_loop,
        },
    })
}

fn validate_render_options(options: &MmlRenderOptions) -> Result<(), MmlRenderError> {
    if !matches!(options.sample_rate_hz, 16_000 | 22_050 | 44_100) {
        return Err(MmlRenderError::UnsupportedSampleRate);
    }
    if matches!(options.seconds, Some(0)) {
        return Err(MmlRenderError::InvalidSeconds);
    }
    if let Some(seconds) = options.seconds {
        let sample_count = u64::from(seconds)
            .checked_mul(u64::from(options.sample_rate_hz))
            .ok_or(MmlRenderError::InvalidSeconds)?;
        if sample_count > u64::from(u32::MAX) {
            return Err(MmlRenderError::InvalidSeconds);
        }
    }
    Ok(())
}

fn render_compact_sequence_table(
    table: &CompactSequenceTable,
    options: &MmlRenderOptions,
) -> Result<Vec<i16>, MmlRenderError> {
    let runtime_tracks: Vec<CompactTrack<'_>> = table
        .tracks
        .iter()
        .map(|track| CompactTrack::new(&track.events, track.gain, track.initial_instrument_id))
        .collect();
    let compact = CompactSequence::new(&table.instruments, &runtime_tracks, table.tempo);
    validate_compact_sequence(compact).map_err(MmlRenderError::RuntimeValidation)?;

    let infinite_loop = compact_table_has_infinite_loop(table);
    if infinite_loop && options.seconds.is_none() {
        return Err(MmlRenderError::InfiniteLoopNeedsSeconds);
    }

    let mut event_storage: Vec<Vec<SequenceEvent>> = table
        .tracks
        .iter()
        .map(|track| vec![SequenceEvent::End; track.events.len()])
        .collect();
    for (track, events_out) in runtime_tracks.iter().zip(event_storage.iter_mut()) {
        track
            .adapt_to_sequence(compact, events_out)
            .map_err(MmlRenderError::RuntimeValidation)?;
    }

    let sequence_limits = AudioLimits {
        sample_rate_hz: options.sample_rate_hz,
        ..AudioLimits::v0_default()
    };
    let sequences: Vec<Sequence<'_>> = event_storage
        .iter()
        .map(|events| {
            let sequence = Sequence::new(
                events,
                koto_audio::BUILTIN_SEQUENCE_INSTRUMENTS.as_slice(),
                compact.tempo.tick_rate_hz,
            );
            sequence
                .validate(sequence_limits)
                .map_err(MmlRenderError::Runtime)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let voices: Vec<PolyphonicSequenceVoice<'_>> = sequences
        .iter()
        .zip(runtime_tracks.iter())
        .map(|(sequence, track)| PolyphonicSequenceVoice::new(*sequence, track.gain))
        .collect();
    let poly = PolyphonicSequence::new(&voices);

    const RENDER_BLOCK_FRAMES: usize = 128;
    let limits = AudioLimits {
        sample_rate_hz: options.sample_rate_hz,
        block_frames: RENDER_BLOCK_FRAMES as u16,
        max_sfx_sources: 4,
        source_queue_depth: 4,
        event_queue_depth: 16,
    };
    let policy = AudioPolicy {
        limits,
        ..AudioPolicy::v0_default()
    };
    let capture = Rc::new(RefCell::new(Vec::new()));
    let backend = VecCaptureBackend::<RENDER_BLOCK_FRAMES>::new(capture.clone());
    let mut service = AudioService::<_, RENDER_BLOCK_FRAMES, 8, 4, 4, 16>::new(policy, backend)
        .map_err(MmlRenderError::Runtime)?;
    service.start().map_err(MmlRenderError::Runtime)?;
    if options.bgm {
        service
            .play_bgm_sequence(poly)
            .map_err(MmlRenderError::Runtime)?;
    } else {
        service
            .play_poly_sequence(poly)
            .map_err(MmlRenderError::Runtime)?;
    }

    let target_samples = options
        .seconds
        .map(|seconds| seconds as usize * options.sample_rate_hz as usize);
    let max_blocks = target_samples
        .map(|samples| samples.div_ceil(RENDER_BLOCK_FRAMES))
        .unwrap_or(usize::MAX);
    let mut completed = false;
    let mut blocks = 0usize;
    while blocks < max_blocks {
        service.tick().map_err(MmlRenderError::Runtime)?;
        blocks += 1;
        while let Some(event) = service.poll_audio_event() {
            if event.kind == AudioEventKind::Completed {
                completed = true;
            }
        }
        if completed && target_samples.is_none() {
            break;
        }
    }

    let mut samples = capture.borrow().clone();
    if let Some(target_samples) = target_samples {
        samples.truncate(target_samples);
    } else {
        let tail_samples = (u64::from(options.sample_rate_hz)
            * u64::from(options.finite_tail_silence_ms)
            / 1000) as usize;
        samples.extend(std::iter::repeat(0).take(tail_samples));
    }
    Ok(samples)
}

fn compact_table_has_infinite_loop(table: &CompactSequenceTable) -> bool {
    table.tracks.iter().any(|track| {
        track.events.iter().any(|event| {
            matches!(
                event,
                CompactEvent::LoopEnd {
                    repeat_count: SEQUENCE_REPEAT_INFINITE
                }
            )
        })
    })
}

#[derive(Clone, Debug)]
struct VecCaptureBackend<const BLOCK_FRAMES: usize> {
    state: BackendState,
    samples: Rc<RefCell<Vec<i16>>>,
}

impl<const BLOCK_FRAMES: usize> VecCaptureBackend<BLOCK_FRAMES> {
    fn new(samples: Rc<RefCell<Vec<i16>>>) -> Self {
        Self {
            state: BackendState::Stopped,
            samples,
        }
    }
}

impl<const BLOCK_FRAMES: usize> AudioBackend<BLOCK_FRAMES> for VecCaptureBackend<BLOCK_FRAMES> {
    fn start(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        Ok(BackendReport::backend_restart())
    }

    fn stop(&mut self) -> BackendResult {
        self.state = BackendState::Stopped;
        Ok(BackendReport::default())
    }

    fn submit_block(&mut self, block: &MixerBlock<BLOCK_FRAMES>) -> BackendResult {
        if self.state != BackendState::Running {
            return Err(BackendError::NotRunning);
        }
        self.samples
            .borrow_mut()
            .extend_from_slice(block.as_pcm16_mono());
        Ok(BackendReport::submitted_block())
    }

    fn suspend(&mut self) -> BackendResult {
        self.state = BackendState::Suspended;
        Ok(BackendReport::default())
    }

    fn resume(&mut self) -> BackendResult {
        self.state = BackendState::Running;
        Ok(BackendReport::backend_restart())
    }

    fn query_state(&self) -> BackendState {
        self.state
    }
}

#[derive(Clone, Debug, PartialEq)]
struct EncodedClip {
    codec_id: CodecId,
    codec_name: &'static str,
    experimental: bool,
    payload: Vec<u8>,
    payload_bytes: u32,
    metrics: Option<Sldpcm4Metrics>,
    loop_validation_result: String,
    decision: ConverterDecision,
    fallback_codec: Option<&'static str>,
    sldpcm4_table_id: Option<&'static str>,
    warnings: Vec<String>,
}

impl EncodedClip {
    fn pcm16(payload: &[u8], sample_count: u32) -> Self {
        let payload_bytes = u32::try_from(payload.len()).unwrap_or(u32::MAX);
        Self {
            codec_id: CodecId::Pcm16,
            codec_name: "PCM16",
            experimental: false,
            payload: payload.to_vec(),
            payload_bytes,
            metrics: None,
            loop_validation_result: if sample_count == 0 {
                "runtime validation".to_string()
            } else {
                "ok".to_string()
            },
            decision: ConverterDecision::Accept,
            fallback_codec: None,
            sldpcm4_table_id: None,
            warnings: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Sldpcm4Metrics {
    peak_absolute_error: u16,
    rms_error: f32,
    snr_db: f32,
    low_signal_snr_reference: bool,
    saturation_count: u32,
}

const SLDPCM4_DELTAS: [i16; 16] = [
    -32768, -16384, -8192, -4096, -2048, -1024, -512, -256, 0, 256, 512, 1024, 2048, 4096, 8192,
    16384,
];

fn encode_experimental_sldpcm4_or_fallback(
    pcm16_payload: &[u8],
    reference_samples: &[i16],
    sample_count: u32,
    loop_metadata: ClipLoop,
    fallback: Sldpcm4FallbackPolicy,
) -> Result<EncodedClip, ConvertError> {
    let loop_validation = validate_sldpcm4_loop(loop_metadata);
    if let Err(reason) = loop_validation {
        return match fallback {
            Sldpcm4FallbackPolicy::Pcm16 => {
                let mut encoded = EncodedClip::pcm16(pcm16_payload, sample_count);
                encoded.loop_validation_result = reason.to_string();
                encoded.decision = ConverterDecision::Fallback;
                encoded.fallback_codec = Some("PCM16");
                encoded
                    .warnings
                    .push(format!("experimental SLDPCM4 fallback to PCM16: {reason}"));
                Ok(encoded)
            }
            Sldpcm4FallbackPolicy::Reject | Sldpcm4FallbackPolicy::ForceExperimental => {
                Err(ConvertError::ExperimentalCodecRejected(reason))
            }
        };
    }

    let encoded = encode_sldpcm4(reference_samples);
    let metrics = compute_sldpcm4_metrics(
        reference_samples,
        &encoded.reconstructed,
        encoded.saturation_count,
    );
    let payload_bytes = u32::try_from(encoded.payload.len())
        .map_err(|_| ConvertError::UnsupportedWav("SLDPCM4 payload is too large"))?;
    let mut warnings =
        vec!["experimental SLDPCM4 is not v0 required and needs listening validation".to_string()];
    if metrics.low_signal_snr_reference {
        warnings.push("SNR is a low-signal reference value".to_string());
    }

    Ok(EncodedClip {
        codec_id: CodecId::Sldpcm4,
        codec_name: "SLDPCM4",
        experimental: true,
        payload: encoded.payload,
        payload_bytes,
        metrics: Some(metrics),
        loop_validation_result: "ok".to_string(),
        decision: ConverterDecision::NeedsListening,
        fallback_codec: None,
        sldpcm4_table_id: Some("standard16"),
        warnings,
    })
}

fn validate_sldpcm4_loop(loop_metadata: ClipLoop) -> Result<(), &'static str> {
    match loop_metadata {
        ClipLoop::None | ClipLoop::Whole { .. } => Ok(()),
        ClipLoop::Forward { start: 0, .. } => Ok(()),
        ClipLoop::Forward { .. } => Err("non-zero forward loop requires PCM16 fallback"),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Sldpcm4EncodeOutput {
    payload: Vec<u8>,
    reconstructed: Vec<i16>,
    saturation_count: u32,
}

fn encode_sldpcm4(samples: &[i16]) -> Sldpcm4EncodeOutput {
    let mut payload = Vec::with_capacity((samples.len() + 1) / 2);
    let mut reconstructed = Vec::with_capacity(samples.len());
    let mut previous = 0i16;
    let mut pending_high = None;
    let mut saturation_count = 0u32;

    for &target in samples {
        let (nibble, sample, saturated) = best_sldpcm4_nibble(previous, target);
        if let Some(high) = pending_high.take() {
            payload.push((high << 4) | nibble);
        } else {
            pending_high = Some(nibble);
        }
        previous = sample;
        reconstructed.push(sample);
        if saturated {
            saturation_count = saturation_count.saturating_add(1);
        }
    }

    if let Some(high) = pending_high {
        payload.push(high << 4);
    }

    Sldpcm4EncodeOutput {
        payload,
        reconstructed,
        saturation_count,
    }
}

fn best_sldpcm4_nibble(previous: i16, target: i16) -> (u8, i16, bool) {
    let mut best_nibble = 0u8;
    let mut best_sample = previous.saturating_add(SLDPCM4_DELTAS[0]);
    let mut best_error = i32::from(target).abs_diff(i32::from(best_sample));
    let mut best_saturated = saturated_add_i16(previous, SLDPCM4_DELTAS[0]).1;

    for (nibble, &delta) in SLDPCM4_DELTAS.iter().enumerate().skip(1) {
        let (sample, saturated) = saturated_add_i16(previous, delta);
        let error = i32::from(target).abs_diff(i32::from(sample));
        if error < best_error {
            best_nibble = nibble as u8;
            best_sample = sample;
            best_error = error;
            best_saturated = saturated;
        }
    }

    (best_nibble, best_sample, best_saturated)
}

fn saturated_add_i16(previous: i16, delta: i16) -> (i16, bool) {
    let sum = i32::from(previous) + i32::from(delta);
    if sum > i32::from(i16::MAX) {
        (i16::MAX, true)
    } else if sum < i32::from(i16::MIN) {
        (i16::MIN, true)
    } else {
        (sum as i16, false)
    }
}

fn compute_sldpcm4_metrics(
    reference: &[i16],
    reconstructed: &[i16],
    saturation_count: u32,
) -> Sldpcm4Metrics {
    let mut peak = 0u16;
    let mut error_square_sum = 0f64;
    let mut signal_square_sum = 0f64;

    for (&expected, &actual) in reference.iter().zip(reconstructed.iter()) {
        let error = i32::from(expected) - i32::from(actual);
        peak = peak.max(error.unsigned_abs().min(u32::from(u16::MAX)) as u16);
        let error = f64::from(error);
        let expected = f64::from(expected);
        error_square_sum += error * error;
        signal_square_sum += expected * expected;
    }

    let count = reference.len().max(1) as f64;
    let rms_error = (error_square_sum / count).sqrt() as f32;
    let low_signal_snr_reference = signal_square_sum < count;
    let snr_db = if error_square_sum == 0.0 {
        f32::INFINITY
    } else {
        (10.0 * (signal_square_sum.max(1.0) / error_square_sum).log10()) as f32
    };

    Sldpcm4Metrics {
        peak_absolute_error: peak,
        rms_error,
        snr_db,
        low_signal_snr_reference,
        saturation_count,
    }
}

fn pcm16_payload_from_samples(samples: &[i16]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

fn decode_pcm16_payload(payload: &[u8]) -> Vec<i16> {
    payload
        .chunks_exact(2)
        .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
        .collect()
}

#[cfg(feature = "experimental-sldpcm4")]
fn decode_sldpcm4_payload(payload: &[u8], sample_count: u32) -> Vec<i16> {
    let sample_count = usize::try_from(sample_count).unwrap_or(usize::MAX);
    let mut samples = Vec::with_capacity(sample_count.min(payload.len().saturating_mul(2)));
    let mut previous = 0i16;

    for index in 0..sample_count {
        let byte = payload[index / 2];
        let nibble = if index % 2 == 0 {
            byte >> 4
        } else {
            byte & 0x0f
        };
        previous = previous.saturating_add(SLDPCM4_DELTAS[usize::from(nibble)]);
        samples.push(previous);
    }

    samples
}

fn write_pcm16_mono_wav(samples: &[i16], sample_rate_hz: u32) -> Result<Vec<u8>, DecodeError> {
    let data_bytes = samples
        .len()
        .checked_mul(2)
        .ok_or(DecodeError::WavTooLarge)?;
    let data_bytes_u32 = u32::try_from(data_bytes).map_err(|_| DecodeError::WavTooLarge)?;
    let riff_size = 36u32
        .checked_add(data_bytes_u32)
        .ok_or(DecodeError::WavTooLarge)?;
    let byte_rate = sample_rate_hz
        .checked_mul(2)
        .ok_or(DecodeError::WavTooLarge)?;

    let mut bytes = Vec::with_capacity(44 + data_bytes);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&riff_size.to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate_hz.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes_u32.to_le_bytes());
    bytes.extend_from_slice(&pcm16_payload_from_samples(samples));
    Ok(bytes)
}

fn wav_frame_count(wav: WavData<'_>) -> Result<u32, ConvertError> {
    let frame_bytes = usize::from(wav.channels) * 2;
    if frame_bytes == 0 {
        return Err(ConvertError::UnsupportedWav(
            "WAV channel count must be non-zero",
        ));
    }
    if wav.data.len() % frame_bytes != 0 {
        return Err(ConvertError::InvalidWav(
            "PCM16 data length is not frame-aligned",
        ));
    }
    u32::try_from(wav.data.len() / frame_bytes)
        .map_err(|_| ConvertError::UnsupportedWav("WAV is too large"))
}

fn wav_mono_samples(wav: WavData<'_>) -> Result<Vec<i16>, ConvertError> {
    let channels = usize::from(wav.channels);
    if channels == 0 {
        return Err(ConvertError::UnsupportedWav(
            "WAV channel count must be non-zero",
        ));
    }
    let frame_bytes = channels * 2;
    if wav.data.len() % frame_bytes != 0 {
        return Err(ConvertError::InvalidWav(
            "PCM16 data length is not frame-aligned",
        ));
    }

    let mut samples = Vec::with_capacity(wav.data.len() / frame_bytes);
    for frame in wav.data.chunks_exact(frame_bytes) {
        let mut sum = 0i64;
        for channel in 0..channels {
            let offset = channel * 2;
            let sample = i16::from_le_bytes([frame[offset], frame[offset + 1]]);
            sum += i64::from(sample);
        }
        samples
            .push((sum / channels as i64).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16);
    }

    Ok(samples)
}

fn resample_linear(
    samples: &[i16],
    source_rate_hz: u32,
    target_rate_hz: u32,
) -> Result<Vec<i16>, ConvertError> {
    if source_rate_hz == 0 || target_rate_hz == 0 {
        return Err(ConvertError::UnsupportedWav(
            "source and target sample rates must be non-zero",
        ));
    }
    if samples.is_empty() {
        return Ok(Vec::new());
    }

    let output_len: usize = ((samples.len() as u128 * u128::from(target_rate_hz)
        + u128::from(source_rate_hz / 2))
        / u128::from(source_rate_hz))
    .try_into()
    .map_err(|_| ConvertError::UnsupportedWav("resampled WAV is too large"))?;
    if output_len == 0 {
        return Ok(Vec::new());
    }

    let mut out = Vec::with_capacity(output_len);
    let ratio = f64::from(source_rate_hz) / f64::from(target_rate_hz);
    for index in 0..output_len {
        let source_pos = index as f64 * ratio;
        let base = source_pos.floor() as usize;
        let frac = source_pos - base as f64;
        let a = f64::from(samples[base.min(samples.len() - 1)]);
        let b = f64::from(samples[(base + 1).min(samples.len() - 1)]);
        let sample = a + (b - a) * frac;
        out.push((sample.round() as i32).clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16);
    }

    Ok(out)
}

fn valid_rust_static_symbol(symbol: &str) -> bool {
    let mut chars = symbol.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn format_drum_table(symbol: &str, samples: &[i16], sample_rate_hz: u32) -> String {
    let mut out = String::new();
    out.push_str("// Generated by koto-audio-tools from license-verified PCM16 mono WAV.\n");
    out.push_str("// Runtime representation: signed PCM16 mono static slice.\n");
    out.push_str(&format!("// Sample rate: {sample_rate_hz} Hz.\n"));
    out.push_str(&format!("pub static {symbol}: &[i16] = &[\n"));
    for chunk in samples.chunks(12) {
        out.push_str("    ");
        for (index, sample) in chunk.iter().enumerate() {
            if index > 0 {
                out.push(' ');
            }
            out.push_str(&sample.to_string());
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("];\n");
    out
}

/// One drum table parsed from a generated PCM16 drum-table module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedDrumTable {
    /// The `pub static` symbol name, such as `BUILTIN_DRUM_BASS_DRUM`.
    pub symbol: String,
    /// PCM16 mono samples parsed from the table body.
    pub samples: Vec<i16>,
}

/// Parse failure for a generated PCM16 drum-table module.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DrumTableParseError {
    /// No `pub static NAME: &[i16]` tables were found in the input.
    MissingTables,
    /// A table body was not terminated with `];`.
    UnterminatedTable,
    /// A table body entry did not parse as an `i16`.
    BadSample(String),
}

/// Per-drum conversion report from [`generate_sldpcm4_drum_table_module`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrumSldpcm4Report {
    /// Source PCM16 symbol name.
    pub symbol: String,
    /// Encoded sample count.
    pub sample_count: u32,
    /// Encoded SLDPCM4 payload bytes (two samples per byte).
    pub payload_bytes: u32,
    /// Saturating reconstructions observed while encoding.
    pub saturation_count: u32,
}

/// Output of [`generate_sldpcm4_drum_table_module`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrumSldpcm4TableOutput {
    /// Complete generated Rust module source.
    pub rust_module: String,
    /// Per-drum conversion reports, in input order.
    pub reports: Vec<DrumSldpcm4Report>,
}

/// Parses the `pub static NAME: &[i16] = &[ ... ];` tables of a generated
/// PCM16 drum-table module (the exact shape `format_drum_table` emits). Using
/// the vendored PCM16 module as the encode source keeps the SLDPCM4 tables in
/// lockstep with whatever drum data is actually in the tree, rather than with
/// the WAVs those tables were once generated from.
pub fn parse_pcm16_drum_table_module(
    source: &str,
) -> Result<Vec<ParsedDrumTable>, DrumTableParseError> {
    let mut tables = Vec::new();
    let mut lines = source.lines();
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("pub static ") else {
            continue;
        };
        let Some((symbol, tail)) = rest.split_once(':') else {
            continue;
        };
        if !tail.trim_start().starts_with("&[i16]") {
            continue;
        }
        let mut samples = Vec::new();
        let mut terminated = false;
        for body_line in lines.by_ref() {
            let body = body_line.trim();
            if body.starts_with("];") {
                terminated = true;
                break;
            }
            for token in body.split(',') {
                let token = token.trim();
                if token.is_empty() {
                    continue;
                }
                let sample = token
                    .parse::<i16>()
                    .map_err(|_| DrumTableParseError::BadSample(token.to_string()))?;
                samples.push(sample);
            }
        }
        if !terminated {
            return Err(DrumTableParseError::UnterminatedTable);
        }
        tables.push(ParsedDrumTable {
            symbol: symbol.trim().to_string(),
            samples,
        });
    }
    if tables.is_empty() {
        return Err(DrumTableParseError::MissingTables);
    }
    Ok(tables)
}

/// Re-encodes parsed PCM16 drum tables as SLDPCM4 payload statics for the
/// `sldpcm4-drums` runtime feature: for each `NAME` this emits
/// `NAME_SLDPCM4: &[u8]` plus `NAME_SAMPLE_COUNT: u32`.
pub fn generate_sldpcm4_drum_table_module(tables: &[ParsedDrumTable]) -> DrumSldpcm4TableOutput {
    let mut out = String::new();
    out.push_str(concat!(
        "// Generated by koto-audio-tools (koto-audio-drum-sldpcm4-table) from the\n",
        "// vendored PCM16 builtin drum tables. Runtime representation: SLDPCM4 nibble\n",
        "// payload (KotoAudioExperimentalStandardLikeV0 delta table, high nibble\n",
        "// first) plus the decoded sample count. Sample rate: 16000 Hz.\n",
    ));
    let mut reports = Vec::with_capacity(tables.len());
    for table in tables {
        let encoded = encode_sldpcm4(&table.samples);
        out.push_str(&format!(
            "pub static {}_SLDPCM4: &[u8] = &[\n",
            table.symbol
        ));
        for chunk in encoded.payload.chunks(16) {
            out.push_str("    ");
            for (index, byte) in chunk.iter().enumerate() {
                if index > 0 {
                    out.push(' ');
                }
                out.push_str(&format!("0x{byte:02x},"));
            }
            out.push('\n');
        }
        out.push_str("];\n");
        out.push_str(&format!(
            "pub const {}_SAMPLE_COUNT: u32 = {};\n",
            table.symbol,
            table.samples.len()
        ));
        reports.push(DrumSldpcm4Report {
            symbol: table.symbol.clone(),
            sample_count: table.samples.len() as u32,
            payload_bytes: encoded.payload.len() as u32,
            saturation_count: encoded.saturation_count,
        });
    }
    DrumSldpcm4TableOutput {
        rust_module: out,
        reports,
    }
}

fn format_compact_sequence_fragment(
    table: &CompactSequenceTable,
    options: &CompactSequenceTableOptions,
) -> String {
    let prefix = &options.table_symbol_prefix;
    let instrument_symbol = format!("{prefix}_INSTRUMENTS");
    let track_symbol = format!("{prefix}_TRACKS");
    let mut out = String::new();

    out.push_str("// Generated by koto-audio-tools from a validated compact sequence table.\n");
    out.push_str("// Runtime representation: borrowed CompactSequence static tables.\n");
    out.push_str(
        "// Experimental MML subset parsing is tools-side; .kmml loading remains future work.\n",
    );
    out.push_str(&format!(
        "pub static {instrument_symbol}: [CompactInstrument; {}] = [\n",
        table.instruments.len()
    ));
    for instrument in &table.instruments {
        out.push_str(&format!(
            "    CompactInstrument {{ builtin_id: {}, volume: {}, attack_ticks: {}, release_ticks: {}, decay_ticks: {} }},\n",
            instrument.builtin_id,
            instrument.volume,
            instrument.attack_ticks,
            instrument.release_ticks,
            instrument.decay_ticks
        ));
    }
    out.push_str("];\n\n");

    for (index, track) in table.tracks.iter().enumerate() {
        out.push_str(&format!(
            "pub static {prefix}_TRACK_{index}_EVENTS: [CompactEvent; {}] = [\n",
            track.events.len()
        ));
        for event in &track.events {
            format_compact_event(&mut out, event);
        }
        out.push_str("];\n\n");
    }

    out.push_str(&format!(
        "pub static {track_symbol}: [CompactTrack<'static>; {}] = [\n",
        table.tracks.len()
    ));
    for (index, track) in table.tracks.iter().enumerate() {
        out.push_str(&format!(
            "    CompactTrack::new(&{prefix}_TRACK_{index}_EVENTS, MixerVolume::new({}), {}),\n",
            track.gain.get(),
            track.initial_instrument_id
        ));
    }
    out.push_str("];\n\n");

    out.push_str(&format!(
        "pub static {}: CompactSequence<'static> = CompactSequence::new(\n",
        options.sequence_symbol_name
    ));
    out.push_str(&format!("    &{instrument_symbol},\n"));
    out.push_str(&format!("    &{track_symbol},\n"));
    out.push_str(&format!(
        "    CompactTempo {{ tick_rate_hz: {}, bpm: {}, ticks_per_beat: {} }},\n",
        table.tempo.tick_rate_hz, table.tempo.bpm, table.tempo.ticks_per_beat
    ));
    out.push_str(");\n");

    out
}

fn format_compact_event(out: &mut String, event: &CompactEvent) {
    match *event {
        CompactEvent::Note {
            pitch,
            duration_ticks,
            volume,
            instrument_id,
        } => out.push_str(&format!(
            "    CompactEvent::Note {{ pitch: {pitch}, duration_ticks: {duration_ticks}, volume: {volume}, instrument_id: {instrument_id} }},\n"
        )),
        CompactEvent::Rest { duration_ticks } => out.push_str(&format!(
            "    CompactEvent::Rest {{ duration_ticks: {duration_ticks} }},\n"
        )),
        CompactEvent::LoopStart => out.push_str("    CompactEvent::LoopStart,\n"),
        CompactEvent::LoopEnd { repeat_count } => out.push_str(&format!(
            "    CompactEvent::LoopEnd {{ repeat_count: {repeat_count} }},\n"
        )),
        CompactEvent::End => out.push_str("    CompactEvent::End,\n"),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WavData<'a> {
    sample_rate_hz: u32,
    channels: u16,
    bits_per_sample: u16,
    data: &'a [u8],
}

fn parse_wav(bytes: &[u8]) -> Result<WavData<'_>, ConvertError> {
    if bytes.len() < 12 {
        return Err(ConvertError::InvalidWav("missing RIFF header"));
    }
    if &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(ConvertError::InvalidWav("not a RIFF/WAVE file"));
    }

    let mut offset = 12usize;
    let mut fmt = None;
    let mut data = None;
    while offset.checked_add(8).is_some_and(|end| end <= bytes.len()) {
        let id = &bytes[offset..offset + 4];
        let len = read_u32(bytes, offset + 4) as usize;
        let start = offset + 8;
        let end = start
            .checked_add(len)
            .ok_or(ConvertError::InvalidWav("chunk length overflows"))?;
        if end > bytes.len() {
            return Err(ConvertError::InvalidWav("chunk extends past file"));
        }

        match id {
            b"fmt " => fmt = Some(parse_fmt(&bytes[start..end])?),
            b"data" => data = Some(&bytes[start..end]),
            _ => {}
        }

        offset = end + (len & 1);
    }

    let fmt = fmt.ok_or(ConvertError::InvalidWav("missing fmt chunk"))?;
    let data = data.ok_or(ConvertError::InvalidWav("missing data chunk"))?;
    if data.len() % 2 != 0 {
        return Err(ConvertError::InvalidWav("PCM16 data has odd byte length"));
    }

    Ok(WavData {
        sample_rate_hz: fmt.sample_rate_hz,
        channels: fmt.channels,
        bits_per_sample: fmt.bits_per_sample,
        data,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FmtChunk {
    sample_rate_hz: u32,
    channels: u16,
    bits_per_sample: u16,
}

fn parse_fmt(bytes: &[u8]) -> Result<FmtChunk, ConvertError> {
    if bytes.len() < 16 {
        return Err(ConvertError::InvalidWav("fmt chunk is too small"));
    }
    let audio_format = read_u16(bytes, 0);
    if audio_format != 1 {
        return Err(ConvertError::UnsupportedWav(
            "only PCM WAV format is supported",
        ));
    }

    Ok(FmtChunk {
        channels: read_u16(bytes, 2),
        sample_rate_hz: read_u32(bytes, 4),
        bits_per_sample: read_u16(bytes, 14),
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use koto_audio::{
        AudioError, BUILTIN_INSTRUMENT_CLOSED_HI_HAT, BUILTIN_INSTRUMENT_SQUARE,
        CLIP_ASSET_HEADER_SIZE,
    };

    fn wav(channels: u16, sample_rate_hz: u32, bits_per_sample: u16, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let byte_rate = sample_rate_hz * u32::from(channels) * u32::from(bits_per_sample) / 8;
        let block_align = channels * bits_per_sample / 8;
        let riff_size = 36 + payload.len() as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&riff_size.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate_hz.to_le_bytes());
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&block_align.to_le_bytes());
        bytes.extend_from_slice(&bits_per_sample.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        bytes.extend_from_slice(payload);
        bytes
    }

    fn pcm16_payload(samples: &[i16]) -> Vec<u8> {
        samples
            .iter()
            .flat_map(|sample| sample.to_le_bytes())
            .collect()
    }

    #[test]
    fn converter_accepts_pcm16_mono_fixture_and_runtime_accepts_asset() {
        let payload = [1, 0, 255, 255];
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &payload);

        let output =
            convert_wav_to_clip_asset("fixture.wav", &input, ConvertOptions::default()).unwrap();
        let clip = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap();

        assert_eq!(clip.payload, payload);
        assert_eq!(clip.sample_count, 2);
        assert_eq!(clip.codec, CodecId::Pcm16);
        assert_eq!(output.report.codec, "PCM16");
        assert!(!output.report.experimental);
        assert_eq!(
            output.asset_bytes.len(),
            CLIP_ASSET_HEADER_SIZE + payload.len()
        );
        assert_eq!(output.report.validation_result, Ok(()));
        assert!(output
            .report
            .to_human_readable()
            .contains("validation result: ok"));
        assert!(output
            .report
            .to_human_readable()
            .contains("downmix applied: false"));
        assert!(output
            .report
            .to_human_readable()
            .contains("resample applied: false"));
    }

    #[test]
    fn drum_table_generator_formats_static_pcm16_slice() {
        let fragment = format_drum_table("DRUM_BD", &[0, -2, 32767], 16_000);

        assert!(fragment.contains("pub static DRUM_BD: &[i16] = &["));
        assert!(fragment.contains("Sample rate: 16000 Hz."));
        assert!(fragment.contains("    0, -2, 32767,"));
        assert!(fragment.contains("Runtime representation: signed PCM16 mono static slice"));
        assert!(fragment.ends_with("];\n"));
    }

    #[test]
    fn drum_table_generator_reads_small_pcm16_mono_wav() {
        let input = wav(
            1,
            AudioLimits::v0_default().sample_rate_hz,
            16,
            &pcm16_payload(&[100, -200, 300]),
        );
        let output = generate_drum_table_from_wav(
            &input,
            DrumTableOptions {
                symbol_name: "DRUM_TEST".to_string(),
                ..DrumTableOptions::default()
            },
        )
        .unwrap();

        assert_eq!(
            output.sample_rate_hz,
            AudioLimits::v0_default().sample_rate_hz
        );
        assert_eq!(output.sample_count, 3);
        assert!(!output.downmix_applied);
        assert!(!output.resample_applied);
        assert!(output
            .rust_fragment
            .contains("pub static DRUM_TEST: &[i16] = &["));
        assert!(output.rust_fragment.contains("    100, -200, 300,"));
    }

    #[test]
    fn drum_table_generator_rejects_invalid_static_symbol() {
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &[0, 0]);

        assert_eq!(
            generate_drum_table_from_wav(
                &input,
                DrumTableOptions {
                    symbol_name: "1_BAD".to_string(),
                    ..DrumTableOptions::default()
                },
            ),
            Err(ConvertError::InvalidRustSymbol)
        );
    }

    #[test]
    fn compact_sequence_table_formatter_emits_expected_rust_fragment() {
        let table = CompactSequenceTable {
            instruments: vec![
                CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 240),
                CompactInstrument::builtin(BUILTIN_INSTRUMENT_CLOSED_HI_HAT, 180),
            ],
            tracks: vec![CompactSequenceTableTrack {
                events: vec![
                    CompactEvent::LoopStart,
                    CompactEvent::Note {
                        pitch: 262,
                        duration_ticks: 2,
                        volume: 210,
                        instrument_id: 0,
                    },
                    CompactEvent::Rest { duration_ticks: 1 },
                    CompactEvent::Note {
                        pitch: 1,
                        duration_ticks: 1,
                        volume: 255,
                        instrument_id: 1,
                    },
                    CompactEvent::LoopEnd {
                        repeat_count: koto_audio::SEQUENCE_REPEAT_INFINITE,
                    },
                    CompactEvent::End,
                ],
                gain: MixerVolume::new(224),
                initial_instrument_id: 0,
            }],
            tempo: CompactTempo {
                tick_rate_hz: 8,
                bpm: 120,
                ticks_per_beat: 4,
            },
        };

        let output = format_compact_sequence_table(
            &table,
            CompactSequenceTableOptions {
                sequence_symbol_name: "LINE_CLEAR_COMPACT".to_string(),
                table_symbol_prefix: "LINE_CLEAR_COMPACT".to_string(),
            },
        )
        .unwrap();

        assert_eq!(output.instrument_count, 2);
        assert_eq!(output.track_count, 1);
        assert_eq!(output.event_counts, vec![6]);
        assert_eq!(
            output.rust_fragment,
            concat!(
                "// Generated by koto-audio-tools from a validated compact sequence table.\n",
                "// Runtime representation: borrowed CompactSequence static tables.\n",
                "// Experimental MML subset parsing is tools-side; .kmml loading remains future work.\n",
                "pub static LINE_CLEAR_COMPACT_INSTRUMENTS: [CompactInstrument; 2] = [\n",
                "    CompactInstrument { builtin_id: 3, volume: 240, attack_ticks: 0, release_ticks: 0, decay_ticks: 0 },\n",
                "    CompactInstrument { builtin_id: 10, volume: 180, attack_ticks: 0, release_ticks: 0, decay_ticks: 0 },\n",
                "];\n",
                "\n",
                "pub static LINE_CLEAR_COMPACT_TRACK_0_EVENTS: [CompactEvent; 6] = [\n",
                "    CompactEvent::LoopStart,\n",
                "    CompactEvent::Note { pitch: 262, duration_ticks: 2, volume: 210, instrument_id: 0 },\n",
                "    CompactEvent::Rest { duration_ticks: 1 },\n",
                "    CompactEvent::Note { pitch: 1, duration_ticks: 1, volume: 255, instrument_id: 1 },\n",
                "    CompactEvent::LoopEnd { repeat_count: 0 },\n",
                "    CompactEvent::End,\n",
                "];\n",
                "\n",
                "pub static LINE_CLEAR_COMPACT_TRACKS: [CompactTrack<'static>; 1] = [\n",
                "    CompactTrack::new(&LINE_CLEAR_COMPACT_TRACK_0_EVENTS, MixerVolume::new(224), 0),\n",
                "];\n",
                "\n",
                "pub static LINE_CLEAR_COMPACT: CompactSequence<'static> = CompactSequence::new(\n",
                "    &LINE_CLEAR_COMPACT_INSTRUMENTS,\n",
                "    &LINE_CLEAR_COMPACT_TRACKS,\n",
                "    CompactTempo { tick_rate_hz: 8, bpm: 120, ticks_per_beat: 4 },\n",
                ");\n",
            )
        );
    }

    #[test]
    fn compact_sequence_table_validates_before_formatting() {
        let table = CompactSequenceTable {
            instruments: vec![CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 255)],
            tracks: vec![CompactSequenceTableTrack {
                events: vec![
                    CompactEvent::Note {
                        pitch: 440,
                        duration_ticks: 1,
                        volume: 255,
                        instrument_id: 0,
                    },
                    CompactEvent::End,
                ],
                gain: MixerVolume::UNITY,
                initial_instrument_id: 0,
            }],
            tempo: CompactTempo::from_tick_rate_hz(4),
        };

        let output = format_compact_sequence_table(
            &table,
            CompactSequenceTableOptions {
                sequence_symbol_name: "VALID_COMPACT".to_string(),
                table_symbol_prefix: "VALID_COMPACT".to_string(),
            },
        )
        .unwrap();

        assert!(output.rust_fragment.contains("pub static VALID_COMPACT"));
    }

    #[test]
    fn compact_sequence_table_invalid_sequence_reports_runtime_validation_error() {
        let table = CompactSequenceTable {
            instruments: vec![CompactInstrument::builtin(BUILTIN_INSTRUMENT_SQUARE, 255)],
            tracks: vec![CompactSequenceTableTrack {
                events: vec![CompactEvent::Rest { duration_ticks: 0 }, CompactEvent::End],
                gain: MixerVolume::UNITY,
                initial_instrument_id: 0,
            }],
            tempo: CompactTempo::from_tick_rate_hz(4),
        };

        assert_eq!(
            format_compact_sequence_table(&table, CompactSequenceTableOptions::default()),
            Err(CompactSequenceTableError::RuntimeValidation(
                CompactSequenceError::ZeroDuration
            ))
        );
    }

    #[test]
    fn mml_render_blocks_like_bgm_to_pcm16_mono_wav() {
        let source = include_str!("../../../examples/mml/blocks_like_bgm.mml");
        let output = render_mml_to_wav(
            "blocks_like_bgm.mml",
            source,
            MmlRenderOptions {
                seconds: Some(1),
                bgm: true,
                ..MmlRenderOptions::default()
            },
        )
        .unwrap();
        let wav = parse_wav(&output.wav_bytes).unwrap();

        assert_eq!(wav.sample_rate_hz, 22_050);
        assert_eq!(wav.channels, 1);
        assert_eq!(wav.bits_per_sample, 16);
        assert_eq!(wav.data.len(), 22_050 * 2);
        assert!(output.report.bgm);
        assert!(output.report.infinite_loop);
    }

    #[test]
    fn mml_render_line_clear_jingle_finishes_with_tail_and_non_silence() {
        let source = include_str!("../../../examples/mml/line_clear_jingle.mml");
        let output =
            render_mml_to_wav("line_clear_jingle.mml", source, MmlRenderOptions::default())
                .unwrap();
        let wav = parse_wav(&output.wav_bytes).unwrap();
        let samples = decode_pcm16_payload(wav.data);

        assert_eq!(wav.sample_rate_hz, 22_050);
        assert_eq!(wav.channels, 1);
        assert_eq!(wav.bits_per_sample, 16);
        assert!(samples.iter().any(|sample| *sample != 0));
        assert!(samples.ends_with(&[0; 16]));
        assert!(!output.report.seconds_limited);
    }

    #[test]
    fn mml_render_seconds_and_sample_rate_control_sample_count() {
        let output = render_mml_to_wav(
            "fixture.mml",
            "T120 L4 O4 @0 c",
            MmlRenderOptions {
                sample_rate_hz: 16_000,
                seconds: Some(2),
                ..MmlRenderOptions::default()
            },
        )
        .unwrap();
        let wav = parse_wav(&output.wav_bytes).unwrap();

        assert_eq!(wav.sample_rate_hz, 16_000);
        assert_eq!(wav.data.len(), 16_000 * 2 * 2);
        assert_eq!(output.report.sample_count, 32_000);
    }

    #[test]
    fn mml_render_drum_aliases_generate_non_silence() {
        let output = render_mml_to_wav(
            "drums.mml",
            "T120 L16 !bd !hh !sd !oh !cy !th !tm !tl !cl",
            MmlRenderOptions::default(),
        )
        .unwrap();
        let wav = parse_wav(&output.wav_bytes).unwrap();
        let samples = decode_pcm16_payload(wav.data);

        assert!(samples.iter().any(|sample| *sample != 0));
    }

    #[test]
    fn mml_render_invalid_mml_reports_error() {
        assert!(matches!(
            render_mml_to_wav("bad.mml", "!zz", MmlRenderOptions::default()),
            Err(MmlRenderError::Parse(mml::MmlParseError::UnknownDrumAlias(alias)))
                if alias == "zz"
        ));
    }

    #[test]
    fn mml_render_rejects_infinite_loop_without_seconds() {
        assert_eq!(
            render_mml_to_wav("loop.mml", "[c]0", MmlRenderOptions::default()),
            Err(MmlRenderError::InfiniteLoopNeedsSeconds)
        );
    }

    #[test]
    fn mml_render_rejects_unsupported_sample_rate() {
        assert_eq!(
            render_mml_to_wav(
                "rate.mml",
                "c",
                MmlRenderOptions {
                    sample_rate_hz: 48_000,
                    ..MmlRenderOptions::default()
                }
            ),
            Err(MmlRenderError::UnsupportedSampleRate)
        );
    }

    #[test]
    fn converter_resamples_sample_rate_mismatch_fixture_by_default() {
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz + 1, 16, &[0, 0]);

        let output =
            convert_wav_to_clip_asset("resample.wav", &input, ConvertOptions::default()).unwrap();

        assert_eq!(output.report.source_sample_rate_hz, 16001);
        assert_eq!(output.report.output_sample_rate_hz, 16000);
        assert!(output.report.resample_applied);
        assert_eq!(output.report.resampler, "linear");
    }

    #[test]
    fn converter_strict_input_rejects_sample_rate_mismatch_fixture() {
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz + 1, 16, &[0, 0]);
        let options = ConvertOptions {
            strict_input: true,
            ..ConvertOptions::default()
        };

        assert_eq!(
            convert_wav_to_clip_asset("bad.wav", &input, options),
            Err(ConvertError::SampleRateMismatch {
                source: AudioLimits::v0_default().sample_rate_hz + 1,
                expected: AudioLimits::v0_default().sample_rate_hz,
            })
        );
    }

    #[test]
    fn converter_downmixes_stereo_fixture_by_default() {
        let input = wav(
            2,
            AudioLimits::v0_default().sample_rate_hz,
            16,
            &pcm16_payload(&[i16::MAX, i16::MAX]),
        );

        let output =
            convert_wav_to_clip_asset("stereo.wav", &input, ConvertOptions::default()).unwrap();
        let clip = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap();

        assert_eq!(clip.payload, pcm16_payload(&[i16::MAX]));
        assert!(output.report.downmix_applied);
        assert_eq!(output.report.source_channels, 2);
        assert_eq!(output.report.original_frame_count, 1);
        assert_eq!(output.report.output_sample_count, 1);
    }

    #[test]
    fn converter_can_bound_a_one_shot_after_conversion() {
        let payload = pcm16_payload(&[100, 200, 300, 400, 500]);
        let input = wav(1, 16_000, 16, &payload);
        let options = ConvertOptions {
            max_output_samples: Some(3),
            ..ConvertOptions::default()
        };

        let output = convert_wav_to_clip_asset("bounded.wav", &input, options).unwrap();
        let clip = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap();
        assert_eq!(clip.sample_count, 3);
        assert_eq!(clip.payload, pcm16_payload(&[100, 200, 300]));
        assert_eq!(output.report.original_frame_count, 5);
        assert_eq!(output.report.output_sample_count, 3);
    }

    #[test]
    fn converter_strict_input_rejects_stereo_fixture() {
        let input = wav(
            2,
            AudioLimits::v0_default().sample_rate_hz,
            16,
            &[0, 0, 0, 0],
        );
        let options = ConvertOptions {
            strict_input: true,
            ..ConvertOptions::default()
        };

        assert_eq!(
            convert_wav_to_clip_asset("stereo.wav", &input, options),
            Err(ConvertError::NonMono { channels: 2 })
        );
    }

    #[test]
    fn converts_44100_mono_wav_to_16000_kacl() {
        let payload = pcm16_payload(&[0, 1000, 2000, 3000, 4000, 5000, 6000, 7000]);
        let input = wav(1, 44_100, 16, &payload);

        let output =
            convert_wav_to_clip_asset("44100.wav", &input, ConvertOptions::default()).unwrap();
        let clip = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap();

        assert_eq!(clip.sample_rate_hz, 16_000);
        assert_eq!(clip.channels, PCM16_MONO_CHANNELS);
        assert_eq!(clip.codec, CodecId::Pcm16);
        assert_eq!(output.report.source_sample_rate_hz, 44_100);
        assert!(output.report.resample_applied);
        assert!(!output.report.downmix_applied);
        assert_eq!(output.report.resampler, "linear");
    }

    #[test]
    fn converts_48000_stereo_wav_to_16000_mono_kacl() {
        let payload = pcm16_payload(&[1000, -1000, 2000, -2000, 3000, -3000]);
        let input = wav(2, 48_000, 16, &payload);

        let output =
            convert_wav_to_clip_asset("48000-stereo.wav", &input, ConvertOptions::default())
                .unwrap();
        let clip = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap();

        assert_eq!(clip.sample_rate_hz, 16_000);
        assert_eq!(clip.channels, PCM16_MONO_CHANNELS);
        assert_eq!(output.report.source_channels, 2);
        assert!(output.report.downmix_applied);
        assert!(output.report.resample_applied);
    }

    #[test]
    fn stereo_downmix_does_not_wrap() {
        let input = wav(
            2,
            AudioLimits::v0_default().sample_rate_hz,
            16,
            &pcm16_payload(&[i16::MAX, i16::MAX, i16::MIN, i16::MIN]),
        );

        let output =
            convert_wav_to_clip_asset("hot-stereo.wav", &input, ConvertOptions::default()).unwrap();
        let clip = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap();

        assert_eq!(clip.payload, pcm16_payload(&[i16::MAX, i16::MIN]));
    }

    #[test]
    fn resample_output_sample_count_is_reasonable() {
        let payload = pcm16_payload(&[0; 48]);
        let input = wav(1, 48_000, 16, &payload);

        let output =
            convert_wav_to_clip_asset("count.wav", &input, ConvertOptions::default()).unwrap();

        assert_eq!(output.report.original_frame_count, 48);
        assert_eq!(output.report.output_sample_count, 16);
        assert_eq!(output.report.decoded_sample_count, 16);
    }

    #[test]
    fn runtime_rejects_malformed_converter_like_fixture() {
        let payload = [0, 0];
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &payload);
        let mut output =
            convert_wav_to_clip_asset("fixture.wav", &input, ConvertOptions::default()).unwrap();
        output.asset_bytes[0] = b'X';

        let error = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap_err();

        assert_eq!(error, ClipAssetError::InvalidMagic);
        assert_eq!(error.as_audio_error(), AudioError::MalformedAsset);
    }

    #[test]
    fn decodes_pcm16_kacl_to_pcm16_mono_wav() {
        let payload = pcm16_payload(&[-1000, 0, 1000]);
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &payload);
        let output =
            convert_wav_to_clip_asset("fixture.wav", &input, ConvertOptions::default()).unwrap();

        let decoded = decode_clip_asset_to_wav("fixture.kacl", &output.asset_bytes).unwrap();
        let wav = parse_wav(&decoded.wav_bytes).unwrap();

        assert_eq!(wav.sample_rate_hz, AudioLimits::v0_default().sample_rate_hz);
        assert_eq!(wav.channels, 1);
        assert_eq!(wav.bits_per_sample, 16);
        assert_eq!(wav.data, payload);
        assert_eq!(decoded.report.codec, "PCM16");
        assert!(!decoded.report.experimental);
        assert_eq!(decoded.report.sample_count, 3);
        assert!(decoded
            .report
            .to_human_readable()
            .contains("output format: PCM16 mono WAV"));
    }

    #[test]
    fn kacl_decode_reports_malformed_asset() {
        assert_eq!(
            decode_clip_asset_to_wav("broken.kacl", b"not kacl"),
            Err(DecodeError::RuntimeValidation(ClipAssetError::Truncated))
        );
    }

    #[test]
    #[cfg(not(feature = "experimental-sldpcm4"))]
    fn sldpcm4_kacl_decode_is_unsupported_without_feature() {
        let mut bytes = ClipAssetHeader {
            version: koto_audio::CLIP_ASSET_VERSION,
            header_size: koto_audio::CLIP_ASSET_HEADER_SIZE as u16,
            codec: CodecId::Sldpcm4,
            channels: u16::from(PCM16_MONO_CHANNELS),
            sample_rate_hz: AudioLimits::v0_default().sample_rate_hz,
            sample_count: 2,
            loop_start: 0,
            loop_end: 0,
            loop_count: 0,
            payload_size: 1,
            placement: AssetPlacement::Unspecified,
            budget_hint_bytes: 0,
        }
        .encode()
        .to_vec();
        bytes.push(0x88);

        assert_eq!(
            decode_clip_asset_to_wav("sldpcm4.kacl", &bytes),
            Err(DecodeError::RuntimeValidation(
                ClipAssetError::UnsupportedCodec
            ))
        );
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn decodes_sldpcm4_kacl_to_pcm16_mono_wav_with_feature() {
        let mut bytes = ClipAssetHeader {
            version: koto_audio::CLIP_ASSET_VERSION,
            header_size: koto_audio::CLIP_ASSET_HEADER_SIZE as u16,
            codec: CodecId::Sldpcm4,
            channels: u16::from(PCM16_MONO_CHANNELS),
            sample_rate_hz: AudioLimits::v0_default().sample_rate_hz,
            sample_count: 4,
            loop_start: 0,
            loop_end: 0,
            loop_count: 0,
            payload_size: 2,
            placement: AssetPlacement::Unspecified,
            budget_hint_bytes: 0,
        }
        .encode()
        .to_vec();
        bytes.extend_from_slice(&[0xf8, 0x10]);

        let decoded = decode_clip_asset_to_wav("sldpcm4.kacl", &bytes).unwrap();
        let wav = parse_wav(&decoded.wav_bytes).unwrap();

        assert_eq!(wav.sample_rate_hz, AudioLimits::v0_default().sample_rate_hz);
        assert_eq!(wav.channels, 1);
        assert_eq!(wav.bits_per_sample, 16);
        assert_eq!(wav.data, pcm16_payload(&[16384, 16384, 0, -32768]));
        assert_eq!(decoded.report.codec, "SLDPCM4");
        assert!(decoded.report.experimental);
        assert_eq!(decoded.report.sldpcm4_table_id, Some("standard16"));
    }

    #[test]
    fn converter_rejects_invalid_loop_using_runtime_validation() {
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &[0, 0]);
        let options = ConvertOptions {
            loop_metadata: ClipLoop::Forward {
                start: 1,
                end: 1,
                count: koto_audio::LoopCount::Finite(1),
            },
            ..ConvertOptions::default()
        };

        assert_eq!(
            convert_wav_to_clip_asset("bad-loop.wav", &input, options),
            Err(ConvertError::RuntimeValidation(ClipAssetError::InvalidLoop))
        );
    }

    #[test]
    fn sldpcm4_silence_encodes_zero_delta_nibbles() {
        let encoded = encode_sldpcm4(&[0, 0, 0, 0]);

        assert_eq!(encoded.payload, [0x88, 0x88]);
        assert_eq!(encoded.reconstructed, [0, 0, 0, 0]);
        assert_eq!(encoded.saturation_count, 0);
    }

    #[test]
    fn sldpcm4_simple_ramp_and_step_reconstruct_expected_samples() {
        let ramp = encode_sldpcm4(&[256, 512, 1024, 2048]);
        let step = encode_sldpcm4(&[16384, 16384, -16384, -16384]);

        assert_eq!(ramp.payload, [0x99, 0xab]);
        assert_eq!(ramp.reconstructed, [256, 512, 1024, 2048]);
        assert_eq!(step.reconstructed, [16384, 16384, -16384, -16384]);
    }

    #[test]
    fn sldpcm4_payload_size_is_ceil_sample_count_over_two() {
        assert_eq!(encode_sldpcm4(&[]).payload.len(), 0);
        assert_eq!(encode_sldpcm4(&[0]).payload.len(), 1);
        assert_eq!(encode_sldpcm4(&[0, 0]).payload.len(), 1);
        assert_eq!(encode_sldpcm4(&[0, 0, 0]).payload.len(), 2);
    }

    #[test]
    fn sldpcm4_packs_high_nibble_first_and_pads_odd_low_nibble() {
        let encoded = encode_sldpcm4(&[16384, 0, -16384]);

        assert_eq!(encoded.payload, [0xf1, 0x10]);
        assert_eq!(encoded.reconstructed, [16384, 0, -16384]);
    }

    #[test]
    fn sldpcm4_metrics_are_computed_after_decode_equivalent() {
        let reference = [1000, 900, 800];
        let encoded = encode_sldpcm4(&reference);
        let metrics =
            compute_sldpcm4_metrics(&reference, &encoded.reconstructed, encoded.saturation_count);

        assert!(metrics.peak_absolute_error > 0);
        assert!(metrics.rms_error > 0.0);
        assert!(metrics.snr_db.is_finite());
    }

    #[test]
    fn sldpcm4_converter_writes_codec_id_16_when_explicit() {
        let payload = pcm16_payload(&[0, 1, 2]);
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &payload);
        let options = ConvertOptions {
            output_codec: OutputCodec::ExperimentalSldpcm4 {
                fallback: Sldpcm4FallbackPolicy::Pcm16,
            },
            ..ConvertOptions::default()
        };

        let output = convert_wav_to_clip_asset("fixture.wav", &input, options).unwrap();
        let header = koto_audio::ClipAssetHeader::decode(&output.asset_bytes).unwrap();

        assert_eq!(header.codec, CodecId::Sldpcm4);
        assert_eq!(header.payload_size, 2);
        assert_eq!(output.report.codec, "SLDPCM4");
        assert!(output.report.experimental);
        assert_eq!(output.report.sldpcm4_table_id, Some("standard16"));
        assert_eq!(
            output.report.converter_decision,
            ConverterDecision::NeedsListening
        );
        assert!(output
            .report
            .to_human_readable()
            .contains("experimental codec: true"));
        assert!(output
            .report
            .to_human_readable()
            .contains("SLDPCM4 table id: standard16"));
    }

    #[test]
    #[cfg(not(feature = "experimental-sldpcm4"))]
    fn generated_sldpcm4_asset_is_rejected_by_runtime_without_feature() {
        let payload = pcm16_payload(&[0, 1, 2]);
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &payload);
        let options = ConvertOptions {
            output_codec: OutputCodec::ExperimentalSldpcm4 {
                fallback: Sldpcm4FallbackPolicy::Pcm16,
            },
            ..ConvertOptions::default()
        };

        let output = convert_wav_to_clip_asset("fixture.wav", &input, options).unwrap();

        assert_eq!(
            parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()),
            Err(ClipAssetError::UnsupportedCodec)
        );
    }

    #[test]
    #[cfg(feature = "experimental-sldpcm4")]
    fn generated_sldpcm4_asset_is_accepted_by_runtime_with_feature() {
        let payload = pcm16_payload(&[0, 1, 2]);
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &payload);
        let options = ConvertOptions {
            output_codec: OutputCodec::ExperimentalSldpcm4 {
                fallback: Sldpcm4FallbackPolicy::Pcm16,
            },
            ..ConvertOptions::default()
        };

        let output = convert_wav_to_clip_asset("fixture.wav", &input, options).unwrap();
        let clip = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap();

        assert_eq!(clip.codec, CodecId::Sldpcm4);
        assert_eq!(clip.payload.len(), 2);
    }

    #[test]
    fn sldpcm4_non_zero_forward_loop_falls_back_to_pcm16() {
        let payload = pcm16_payload(&[0, 1, 2, 3]);
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &payload);
        let options = ConvertOptions {
            loop_metadata: ClipLoop::Forward {
                start: 1,
                end: 3,
                count: koto_audio::LoopCount::Finite(1),
            },
            output_codec: OutputCodec::ExperimentalSldpcm4 {
                fallback: Sldpcm4FallbackPolicy::Pcm16,
            },
            ..ConvertOptions::default()
        };

        let output = convert_wav_to_clip_asset("loop.wav", &input, options).unwrap();
        let clip = parse_clip_asset(&output.asset_bytes, AudioLimits::v0_default()).unwrap();

        assert_eq!(clip.codec, CodecId::Pcm16);
        assert_eq!(clip.payload, payload);
        assert_eq!(
            output.report.converter_decision,
            ConverterDecision::Fallback
        );
        assert_eq!(output.report.fallback_codec, Some("PCM16"));
        assert!(output
            .report
            .loop_validation_result
            .contains("non-zero forward loop"));
    }

    #[test]
    fn sldpcm4_non_zero_forward_loop_can_reject() {
        let payload = pcm16_payload(&[0, 1, 2, 3]);
        let input = wav(1, AudioLimits::v0_default().sample_rate_hz, 16, &payload);
        let options = ConvertOptions {
            loop_metadata: ClipLoop::Forward {
                start: 1,
                end: 3,
                count: koto_audio::LoopCount::Finite(1),
            },
            output_codec: OutputCodec::ExperimentalSldpcm4 {
                fallback: Sldpcm4FallbackPolicy::Reject,
            },
            ..ConvertOptions::default()
        };

        assert_eq!(
            convert_wav_to_clip_asset("loop.wav", &input, options),
            Err(ConvertError::ExperimentalCodecRejected(
                "non-zero forward loop requires PCM16 fallback"
            ))
        );
    }

    #[test]
    fn runtime_crate_manifest_has_no_wav_or_resampler_dependency() {
        let manifest = include_str!("../../koto-audio/Cargo.toml");

        assert!(!manifest.contains("hound"));
        assert!(!manifest.contains("rubato"));
        assert!(!manifest.contains("samplerate"));
        assert!(!manifest.contains("resampler"));
        assert!(!manifest.contains("wav"));
    }
}
