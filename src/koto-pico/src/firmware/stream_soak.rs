//! KOTO-0227 five-minute `WifiStreamAudio` product-path soak (validation only).
//!
//! Drives the real package streaming pipeline — SD KPA range reads, the
//! permanent CPU0 stream scratch, `StreamingClipDecoder`, and CPU1 PCM
//! submission — for five minutes while the arena-owned CYW43 soak future
//! stages bounded packet-buffer activity. PCM16 and SLDPCM4 KACL assets from
//! `sample_audio_codecs.kpa` alternate for the whole window. This module owns
//! only the stream/report half; the caller owns the residency transition,
//! runtime install/shutdown, and rich-audio reconstruction.

use core::fmt::Write;

use embassy_time::{Instant, Timer};
use embedded_sdmmc::{BlockDevice, File, LfnBuffer, Mode, ShortFileName, VolumeIdx, VolumeManager};
use koto_audio::{
    AudioLimits, ClipAssetHeader, CodecId, StreamingClipDecoder, CLIP_ASSET_HEADER_SIZE,
};

use crate::dashboard::LineBuffer;
use crate::firmware::audio::PicoAudioBackend;
use crate::firmware::audio_scratch::{
    try_with_stream, STREAM_PCM16_BYTES, STREAM_REFILL_FRAMES, STREAM_SLD4_BYTES,
};
use crate::firmware::config::FirmwareClock;
use crate::firmware::diag::{uart_log, uart_write_line};
use crate::firmware::wifi_residency::{
    wifi_lifecycle_phase, wifi_soak_tx_frames, WifiLifecyclePhase, WifiRuntime, WifiRuntimeArena,
};

/// The soak streams the two large KACL assets shipped by this sample package.
const SOAK_PACKAGE_LONG_NAME: &str = "sample_audio_codecs.kpa";
const SOAK_PCM16_ASSET: &str = "audio/sample_audio_pcm16.kacl";
const SOAK_SLD4_ASSET: &str = "audio/sample_audio_sld4.kacl";
/// Five-minute acceptance window (KOTO-0227).
const SOAK_DURATION_MS: u64 = 300_000;
/// Radio bring-up bound before the stream half starts.
const RADIO_READY_TIMEOUT_MS: u64 = 15_000;
/// Progress heartbeat interval so a stalled soak is visible mid-run.
const PROGRESS_INTERVAL_MS: u64 = 30_000;
const MAX_ASSET_PATH: usize = 96;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SoakResult {
    Ok,
    SdError,
    AssetError,
    RadioTimeout,
    StreamError,
}

#[derive(Clone, Copy, Debug)]
pub struct StreamSoakReport {
    pub result: SoakResult,
    pub pcm16_passes: u32,
    pub sld4_passes: u32,
    pub refills: u32,
    pub samples_submitted: u32,
    pub underruns: u32,
    pub drops: u32,
    pub tx_frames: u32,
    pub elapsed_ms: u64,
}

impl StreamSoakReport {
    const fn failed(result: SoakResult) -> Self {
        Self {
            result,
            pcm16_passes: 0,
            sld4_passes: 0,
            refills: 0,
            samples_submitted: 0,
            underruns: 0,
            drops: 0,
            tx_frames: 0,
            elapsed_ms: 0,
        }
    }
}

struct ActiveStream {
    decoder: StreamingClipDecoder,
    codec: CodecId,
    payload_offset: u32,
    payload_size: u32,
    cursor: u32,
}

enum RefillOutcome {
    Idle,
    Refilled,
    Finished,
    Failed,
}

fn read_exact<D>(file: &File<'_, D, FirmwareClock, 4, 4, 1>, dst: &mut [u8]) -> bool
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let mut total = 0;
    while total < dst.len() {
        match file.read(&mut dst[total..]) {
            Ok(0) | Err(_) => return false,
            Ok(count) => total += count,
        }
    }
    true
}

/// Locates one `path` inside an open `KPA1` archive, mirroring the product
/// asset-table walk in `AppHost::package_asset_range`.
fn kpa_asset_range<D>(file: &File<'_, D, FirmwareClock, 4, 4, 1>, path: &str) -> Option<(u32, u32)>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let mut header = [0u8; 64];
    if file.seek_from_start(0).is_err() || !read_exact(file, &mut header) {
        return None;
    }
    if &header[..4] != b"KPA1" {
        return None;
    }
    let entry_count = u32::from_le_bytes(header[16..20].try_into().unwrap_or([0; 4]));
    let table_offset = u32::from_le_bytes(header[20..24].try_into().unwrap_or([0; 4]));
    let strings_offset = u32::from_le_bytes(header[24..28].try_into().unwrap_or([0; 4]));
    let mut record = [0u8; 64];
    let mut entry_path = [0u8; MAX_ASSET_PATH];
    for index in 0..entry_count {
        if file
            .seek_from_start(table_offset.saturating_add(index.saturating_mul(64)))
            .is_err()
            || !read_exact(file, &mut record)
        {
            return None;
        }
        let path_offset = u32::from_le_bytes(record[0..4].try_into().unwrap_or([0; 4]));
        let path_len = u32::from_le_bytes(record[4..8].try_into().unwrap_or([0; 4])) as usize;
        if path_len > entry_path.len() || path_len != path.len() {
            continue;
        }
        if file
            .seek_from_start(strings_offset.saturating_add(path_offset))
            .is_err()
            || !read_exact(file, &mut entry_path[..path_len])
        {
            return None;
        }
        if &entry_path[..path_len] == path.as_bytes() {
            // Record layout per `AppHost::package_asset_range`: the asset data
            // offset/size live at bytes 16..24 of the 64-byte table record.
            let data_offset = u32::from_le_bytes(record[16..20].try_into().unwrap_or([0; 4]));
            let data_size = u32::from_le_bytes(record[20..24].try_into().unwrap_or([0; 4]));
            return Some((data_offset, data_size));
        }
    }
    None
}

/// Reads one KACL header and builds a non-looping streaming decoder for it,
/// mirroring `AppHost::start_streaming_audio_asset`.
fn start_stream<D>(
    file: &File<'_, D, FirmwareClock, 4, 4, 1>,
    asset_offset: u32,
    asset_size: u32,
) -> Option<ActiveStream>
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    let mut bytes = [0u8; CLIP_ASSET_HEADER_SIZE];
    if file.seek_from_start(asset_offset).is_err() || !read_exact(file, &mut bytes) {
        return None;
    }
    let header = ClipAssetHeader::decode(&bytes).ok()?;
    let total_size = u32::from(header.header_size).checked_add(header.payload_size)?;
    if total_size != asset_size {
        return None;
    }
    let mut pass_header = header;
    pass_header.loop_start = 0;
    pass_header.loop_end = 0;
    pass_header.loop_count = 0;
    let decoder = StreamingClipDecoder::from_header(pass_header, AudioLimits::v0_default()).ok()?;
    Some(ActiveStream {
        decoder,
        codec: header.codec,
        payload_offset: asset_offset.saturating_add(u32::from(header.header_size)),
        payload_size: header.payload_size,
        cursor: 0,
    })
}

/// One bounded refill through the permanent stream scratch, mirroring
/// `AppHost::service_audio_stream`.
fn refill_stream<D>(
    audio: &mut PicoAudioBackend,
    file: &File<'_, D, FirmwareClock, 4, 4, 1>,
    stream: &mut ActiveStream,
) -> RefillOutcome
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
{
    if stream.decoder.is_finished() {
        return RefillOutcome::Finished;
    }
    if audio.pcm_free_frames() < STREAM_REFILL_FRAMES {
        return RefillOutcome::Idle;
    }
    let remaining = stream.payload_size.saturating_sub(stream.cursor) as usize;
    if remaining == 0 {
        return RefillOutcome::Failed;
    }
    let codec_bytes = match stream.codec {
        CodecId::Pcm16 => STREAM_PCM16_BYTES,
        CodecId::Sldpcm4 => STREAM_SLD4_BYTES,
        CodecId::Unsupported(_) => 0,
    };
    let encoded_len = remaining.min(codec_bytes);
    let scratch_result = try_with_stream(|encoded, decoded| {
        if file
            .seek_from_start(stream.payload_offset.saturating_add(stream.cursor))
            .is_err()
            || !read_exact(file, &mut encoded[..encoded_len])
        {
            return false;
        }
        let mut encoded_cursor = 0usize;
        let mut decoded_frames = 0usize;
        while decoded_frames < STREAM_REFILL_FRAMES && !stream.decoder.is_finished() {
            let output_len = (STREAM_REFILL_FRAMES - decoded_frames).min(decoded.len());
            let (consumed, written) = stream.decoder.decode_chunk(
                &encoded[encoded_cursor..encoded_len],
                &mut decoded[..output_len],
            );
            if consumed == 0 && written == 0 {
                return false;
            }
            encoded_cursor = encoded_cursor.saturating_add(consumed);
            stream.cursor = stream.cursor.saturating_add(consumed as u32);
            decoded_frames = decoded_frames.saturating_add(written);
            if !matches!(
                audio.submit_pcm_mono_i16(16_000, &decoded[..written]),
                Ok(accepted) if accepted as usize == written
            ) {
                return false;
            }
        }
        true
    });
    if !matches!(scratch_result, Ok(true)) {
        return RefillOutcome::Failed;
    }
    audio.set_pcm_stream_active(true);
    if stream.decoder.is_finished() {
        RefillOutcome::Finished
    } else {
        RefillOutcome::Refilled
    }
}

/// Runs the stream half of the soak. The caller must already own an installed
/// [`WifiRuntime`] whose soak future publishes `RadioReady` and stages packet
/// activity; this function waits for that phase, then alternates the two KACL
/// assets until the window closes. On return (any result) PCM stream promises
/// are cleared; the caller performs shutdown and rich-audio reconstruction.
pub async fn run<D, A>(
    audio: &mut PicoAudioBackend,
    runtime: &mut WifiRuntime<A>,
    volume_mgr: &VolumeManager<D, FirmwareClock>,
    lfn_storage: &mut [u8],
    uart: &mut embassy_rp::uart::UartTx<'_, embassy_rp::uart::Blocking>,
) -> StreamSoakReport
where
    D: BlockDevice,
    D::Error: core::fmt::Debug,
    A: WifiRuntimeArena,
{
    let Ok(volume) = volume_mgr.open_volume(VolumeIdx(0)) else {
        return StreamSoakReport::failed(SoakResult::SdError);
    };
    let Ok(root) = volume.open_root_dir() else {
        return StreamSoakReport::failed(SoakResult::SdError);
    };
    let Ok(apps) = root.open_dir("APPS") else {
        return StreamSoakReport::failed(SoakResult::SdError);
    };
    let mut package_name: Option<ShortFileName> = None;
    let mut lfn = LfnBuffer::new(lfn_storage);
    if apps
        .iterate_dir_lfn(&mut lfn, |entry, long_name| {
            if package_name.is_none()
                && !entry.attributes.is_directory()
                && long_name.is_some_and(|name| name.eq_ignore_ascii_case(SOAK_PACKAGE_LONG_NAME))
            {
                package_name = Some(entry.name.clone());
            }
        })
        .is_err()
    {
        return StreamSoakReport::failed(SoakResult::SdError);
    }
    let Some(package_name) = package_name else {
        uart_log(uart, "phase=227 stream-soak package-not-found\r\n");
        return StreamSoakReport::failed(SoakResult::AssetError);
    };
    let Ok(file) = apps.open_file_in_dir(&package_name, Mode::ReadOnly) else {
        return StreamSoakReport::failed(SoakResult::SdError);
    };
    let Some((pcm16_offset, pcm16_size)) = kpa_asset_range(&file, SOAK_PCM16_ASSET) else {
        uart_log(uart, "phase=227 stream-soak asset-missing codec=pcm16\r\n");
        return StreamSoakReport::failed(SoakResult::AssetError);
    };
    let Some((sld4_offset, sld4_size)) = kpa_asset_range(&file, SOAK_SLD4_ASSET) else {
        uart_log(uart, "phase=227 stream-soak asset-missing codec=sld4\r\n");
        return StreamSoakReport::failed(SoakResult::AssetError);
    };
    let mut line = LineBuffer::new();
    let _ = write!(
        line,
        "phase=227 stream-soak assets pcm16={} sld4={}\r\n",
        pcm16_size, sld4_size
    );
    uart_write_line(uart, &line);

    // Radio bring-up: the soak future publishes RadioReady after CLM init.
    let radio_started = Instant::now();
    loop {
        runtime.service().await;
        if wifi_lifecycle_phase() == WifiLifecyclePhase::RadioReady {
            uart_log(uart, "phase=227 stream-soak radio-ready\r\n");
            break;
        }
        if radio_started.elapsed().as_millis() >= RADIO_READY_TIMEOUT_MS {
            uart_log(uart, "phase=227 stream-soak radio-ready-timeout\r\n");
            return StreamSoakReport::failed(SoakResult::RadioTimeout);
        }
        Timer::after_millis(1).await;
    }

    let baseline = audio.stats();
    let mut report = StreamSoakReport::failed(SoakResult::Ok);
    let Some(mut stream) = start_stream(&file, pcm16_offset, pcm16_size) else {
        return StreamSoakReport::failed(SoakResult::AssetError);
    };
    let started = Instant::now();
    let mut next_progress_ms = PROGRESS_INTERVAL_MS;
    loop {
        let elapsed = started.elapsed().as_millis();
        if elapsed >= SOAK_DURATION_MS {
            report.elapsed_ms = elapsed;
            break;
        }
        runtime.service().await;
        match refill_stream(audio, &file, &mut stream) {
            RefillOutcome::Idle => {}
            RefillOutcome::Refilled => report.refills += 1,
            RefillOutcome::Finished => {
                let (next_offset, next_size) = match stream.codec {
                    CodecId::Pcm16 => {
                        report.pcm16_passes += 1;
                        (sld4_offset, sld4_size)
                    }
                    _ => {
                        report.sld4_passes += 1;
                        (pcm16_offset, pcm16_size)
                    }
                };
                match start_stream(&file, next_offset, next_size) {
                    Some(next) => stream = next,
                    None => report.result = SoakResult::StreamError,
                }
            }
            RefillOutcome::Failed => report.result = SoakResult::StreamError,
        }
        if report.result != SoakResult::Ok {
            report.elapsed_ms = started.elapsed().as_millis();
            break;
        }
        if elapsed >= next_progress_ms {
            next_progress_ms = next_progress_ms.saturating_add(PROGRESS_INTERVAL_MS);
            let stats = audio.stats();
            let mut line = LineBuffer::new();
            let _ = write!(
                line,
                "phase=227 stream-soak t={}s pcm16={} sld4={} underruns={} drops={} tx={}\r\n",
                elapsed / 1_000,
                report.pcm16_passes,
                report.sld4_passes,
                stats.underruns.saturating_sub(baseline.underruns),
                stats.drops.saturating_sub(baseline.drops),
                wifi_soak_tx_frames()
            );
            uart_write_line(uart, &line);
        }
        Timer::after_millis(1).await;
    }
    audio.set_pcm_stream_active(false);
    let stats = audio.stats();
    report.samples_submitted = stats
        .samples_submitted
        .saturating_sub(baseline.samples_submitted);
    report.underruns = stats.underruns.saturating_sub(baseline.underruns);
    report.drops = stats.drops.saturating_sub(baseline.drops);
    report.tx_frames = wifi_soak_tx_frames();
    report
}
