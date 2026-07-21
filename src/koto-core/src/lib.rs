#![cfg_attr(not(test), no_std)]

pub const KOTO_COPYRIGHT_NOTICE: &str = "Copyright 2026 Siska-Tech Lab.";

pub mod audio;
pub mod boot_splash;
pub mod config;
pub mod config_ui;
pub mod config_wifi_ui;
pub mod dirty_tiles;
pub mod fetch;
pub mod font;
pub mod fs;
pub mod hal;
pub mod ime;
pub mod json;
pub mod keymap;
pub mod kotodos;
pub mod kpa;
pub mod layout;
pub mod memo;
pub mod memo_ime;
pub mod mqtt;
pub mod net;
pub mod net_ui;
pub mod package;
pub mod psram;
pub mod raster;
pub mod render;
/// The KotoVM bytecode interpreter, extracted into the standalone `koto-vm`
/// crate. Re-exported here as `koto_core::runtime` so existing consumers
/// (`koto-pico`, `koto-sim`) keep their `koto_core::runtime::*` paths unchanged.
pub use koto_vm as runtime;
pub mod shell;
pub mod skk;
pub mod time;
pub mod ui_abi;
pub mod ui_input;
pub mod ui_render;
pub mod ui_session;
pub mod vault;
pub mod wifi_secrets;

pub use audio::{AudioError, PcmMixer, PcmSliceStream};
pub use boot_splash::{
    splash_progress_rect, splash_step_rect, BootSplash, BootStep, BootStepStatus, SPLASH_STEP_COUNT,
};
pub use config::{
    config_generation_is_newer, config_write_slot, newest_config_slot, ConfigCapability,
    ConfigError, ConfigPageDescriptor, ConfigPageId, ConfigPageRegistry, ConfigService, ConfigSlot,
    ConfigSnapshot, Locale, SntpServer, WifiConfigInputs, CONFIG_FORMAT_MAX_BYTES,
    CONFIG_MAX_PUBLIC_SETTINGS,
};
pub use config_ui::{KotoConfigAction, KotoConfigUi, KOTOCONFIG_SURFACE};
pub use config_wifi_ui::{KotoConfigWifiUi, KOTOCONFIG_WIFI_SURFACE};
pub use dirty_tiles::{coalesce_dirty_tiles, coalesce_rects, TileBand};
#[cfg(feature = "app_fetch_tls_verifier")]
pub use fetch::verify_p256_tls13_certificate_signature;
pub use fetch::{
    encode_fetch_get_request, encode_fetch_get_request_with_injection,
    extract_certificate_spki_der, extract_p256_public_key_from_spki_der, parse_fetch_url,
    parse_manifest_fetch_permission, parse_manifest_fetch_permission_into, release_ipv4_allowed,
    release_ipv6_allowed, AllowlistError, AppContext, AppFetchController, AppFetchService,
    BackendPoll, FetchAllowlist, FetchBackend, FetchDiagnostics, FetchError, FetchOrigin,
    FetchPinSet, FetchPinTable, FetchPoll, FetchRequestId, FetchScheme, FetchTransportCommand,
    FetchTransportMailbox, FetchTransportState, FetchUrlTarget, HttpDecodeProgress,
    HttpDecodeState, HttpResponseDecoder, ManifestFetchError, ManifestFetchPermission, OriginError,
    SpkiDerError, SpkiSha256, UnavailableFetchBackend, FETCH_TRANSPORT_CHUNK_BYTES,
    MAX_FETCH_CERTIFICATE_BYTES, MAX_FETCH_DURATION_MS, MAX_FETCH_HEADER_BYTES, MAX_FETCH_ORIGINS,
    MAX_FETCH_READ_BYTES, MAX_FETCH_TOTAL_BYTES, MAX_FETCH_URL_BYTES, MAX_GLOBAL_FETCH_REQUESTS,
    MAX_SPKI_PINS_PER_ORIGIN,
};
pub use font::{BitmapFont, FontError, Glyph};
pub use fs::{FsError, Sandbox, SandboxPath, MAX_VIRTUAL_PATH_LEN};
pub use hal::{
    AudioBuffer, AudioHal, AudioSource, Buttons, FileHandle, FileMode, FsHal, HalError, InputHal,
    InputState, PixelFormat, PowerHal, PowerState, PsramHal, Rect, Surface, VideoHal,
};
pub use ime::{
    ImeError, ImeOutput, RomajiKanaInput, StickyShift, StickyShiftKey, StickyShiftOutput,
    MAX_ROMAJI_BUFFER,
};
pub use json::{
    JsonDecoder, JsonError, JsonEvent, JsonHostSession, JsonValueKind, JsonValueSkip, SkipProgress,
    MAX_JSON_DEPTH, MAX_JSON_NUMBER_BYTES, MAX_JSON_TOKEN_BYTES,
};
pub use kotodos::{
    KotoDosMode, KOTODOS_GAME_HEIGHT, KOTODOS_GAME_REGION, KOTODOS_SCREEN_HEIGHT,
    KOTODOS_SCREEN_WIDTH, KOTODOS_SURFACE, KOTODOS_UI_HEIGHT, KOTODOS_UI_REGION,
};
pub use kpa::{
    KpaEntry, KpaError, KpaHeader, KpaReader, PreloadWindow, KPA_ENTRY_SIZE,
    KPA_FIRST_ASSET_ALIGNMENT, KPA_FLAG_ENTRY, KPA_FLAG_PRELOAD, KPA_FLAG_SEQUENTIAL,
    KPA_HEADER_SIZE, KPA_MAGIC, KPA_PAYLOAD_ALIGNMENT, KPA_VERSION_MAJOR, KPA_VERSION_MINOR,
};
pub use layout::{CellMetrics, LayoutError, TextLayout, MAX_IME_LINES};
pub use memo::{
    MemoDirty, MemoDirtyLines, MemoEditor, MemoError, MemoMove, MEMO_DEFAULT_CAPACITY,
    MEMO_MAX_DIRTY_LINES,
};
pub use memo_ime::{
    KotoMemoIme, MemoIme, MemoImeError, MemoImeKey, MemoImeLine, MemoImeMode,
    MEMO_IME_CANDIDATE_CAPACITY, MEMO_IME_READING_CAPACITY,
};
pub use mqtt::{
    parse_manifest_mqtt_permission, AppMqttService, BackendMqttPoll, BrokerAllowlist, BrokerError,
    BrokerListError, ManifestMqttError, ManifestMqttPermission, MqttBackend, MqttDecodeError,
    MqttDecodeProgress, MqttError, MqttInbound, MqttMessage, MqttMessageQueue, MqttOrigin,
    MqttPacketDecoder, MqttPacketType, MqttPoll, MqttScheme, MqttSessionId, TopicError,
    TopicFilter, TopicFilterSet, TopicSetError, UnavailableMqttBackend, MAX_GLOBAL_MQTT_SESSIONS,
    MAX_MQTT_BROKERS, MAX_MQTT_HOSTNAME_BYTES, MAX_MQTT_MESSAGE_QUEUE, MAX_MQTT_PACKET_BYTES,
    MAX_MQTT_PAYLOAD_BYTES, MAX_MQTT_TOPIC_BYTES, MAX_MQTT_TOPIC_FILTERS, MQTT_CONNECT_DEADLINE_MS,
    MQTT_KEEPALIVE_SECS, MQTT_PROTOCOL_LEVEL, MQTT_RECONNECT_MAX_MS, MQTT_RECONNECT_MIN_MS,
};
pub use net::{
    CredentialProvider, CredentialView, ForgetOutcome, Generation, HalFault, HalPoll, NetworkError,
    NetworkEvent, NetworkService, NetworkSnapshot, OperationState, RadioState, RawScanResult,
    RegionError, RegulatoryRegion, RequestId, ScanResult, Security, ServiceProgress, Ssid,
    SubmitResult, WifiHal, COMMAND_QUEUE_MAX, CREDENTIAL_MAX_BYTES, CREDENTIAL_MIN_BYTES,
    EVENT_QUEUE_MAX, RETAINED_PROFILES_MAX, SCAN_RESULTS_MAX, SSID_MAX_BYTES, STATUS_HISTORY_MAX,
};
pub use net_ui::{WifiIntent, WifiKey, WifiPageController, WifiPageState};
pub use package::{
    IconError, ManifestError, ManifestFields, PackageIcon, PackageIconStyle, PackageIconTheme,
    PackageInfo, PackageList, PackageManifest, KPA_MANIFEST_FORMAT, KPA_MANIFEST_MIN_VERSION,
    KPA_MANIFEST_VERSION, MAX_APP_ID_LEN, MAX_ENTRY_PATH_LEN, MAX_ICON_PATH_LEN, MAX_NAME_LEN,
    MAX_PACKAGES, MAX_RUNTIME_NAME_LEN, PACKAGE_ICON_1BPP_BYTES, PACKAGE_ICON_HEIGHT,
    PACKAGE_ICON_WIDTH,
};
pub use psram::{PsramBlocks, PsramError, PSRAM_BLOCK_SIZE};
pub use raster::{Canvas, Rgb565};
pub use render::{RenderCommand, RenderCommandList, RenderError, RenderSurface, RenderUpdate};
pub use runtime::{
    debug_map, verify_kbc, verify_kbc_streaming, BytecodeSession, BytecodeVm, CodeSource,
    CodeTileTransition, DebugMap, DebugMapError, HostCallOutcome, HostErrorCode, KbcHeader,
    RuntimeLimits, SessionError, SliceCode, SourceLocation, VerifiedProgram, VerifyError, VmError,
    VmHost, VmInputSnapshot, VmRunResult, HOST_ABI_MAJOR, HOST_ABI_MINOR, KBC_DEBUG_ENTRY_SIZE,
    KBC_DEBUG_HEADER_SIZE, KBC_DEBUG_MAGIC, KBC_DEBUG_VERSION, KBC_HEADER_SIZE, KBC_MAGIC,
    KBC_VERSION_MAJOR, KBC_VERSION_MINOR,
};
pub use shell::{
    ShellAction, ShellClock, ShellCommandId, ShellItem, ShellSound, ShellState, ShellStatusText,
    WifiSignal, SHELL_CLOCK_RECT,
};
pub use skk::{
    Candidates, DictEntry, SkkDictAccess, SkkError, SkkIndex, SkkLeadingIndex, SkkRead, SliceDict,
    WindowedDict, DEFAULT_INDEX_CAPACITY, MAX_LEADING_KEY_BYTES, SKK_LOOKUP_WINDOW_BYTES,
};
pub use time::{
    unix_to_calendar, unix_to_shell_clock, CalendarTime, TimeFailure, TimeService,
    TimeServiceAction, TimeSnapshot, TimeSourceKind, UtcOffset, SNTP_PACKET_BYTES, SNTP_REFRESH_MS,
    SNTP_TIMEOUT_MS,
};
pub use ui_abi::{UiAbiError, UiCapabilities, UI_ABI_HOST_MINOR, UI_CAPABILITIES_BYTES};
pub use ui_render::{paint_ui_damage, ui_damage_commands, CanvasUiPainter, UiRenderError};
pub use ui_session::{
    UiMountError, UiNode, UiPollError, UiSession, UI_DAMAGE_CAPACITY, UI_DATA_CAPACITY,
    UI_EVENT_HEADER_SIZE, UI_EVENT_QUEUE_CAPACITY, UI_MAX_LIST_ROWS, UI_MAX_MOUNT_BYTES,
    UI_MAX_NODES, UI_MAX_OPEN_MODALS, UI_MAX_TEXT_FIELDS, UI_MAX_TEXT_FIELD_BYTES,
    UI_MAX_UPDATE_BYTES, UI_SESSION_SRAM_BYTES,
};
// The vault module shares the concept names `LoadOutcome`, `MediumFault`, and
// `SlotRead` with `wifi_secrets`; those stay reachable as `vault::*` to avoid a
// crate-root collision. Only vault-unique names are re-exported here.
pub use vault::{
    CredentialHandle, CredentialInjection, CredentialKind, GrantInfo, ServiceKind, VaultEndpoint,
    VaultError, VaultMedium, VaultSlot, VaultStore, MAX_ENDPOINT_HOST_BYTES, MAX_GRANTS,
    MAX_HEADER_NAME_BYTES, MAX_SECRET_BYTES, MAX_USERNAME_BYTES, VAULT_MAGIC, VAULT_RECORD_BYTES,
};
pub use wifi_secrets::{
    LoadOutcome, MediumFault, ProfileInfo, SecretError, SecretMedium, SecretSlot, SlotRead,
    WifiSecretStore, WIFI_SECRET_MAGIC, WIFI_SECRET_RECORD_BYTES,
};
