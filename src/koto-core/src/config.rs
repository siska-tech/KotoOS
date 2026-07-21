//! Bounded system configuration model shared by simulator and device adapters.
//!
//! This module deliberately contains no filesystem, UI, VM, or board code.
//! Platform adapters persist complete encoded snapshots; KotoConfig is the only
//! normal mutation surface, while Shell and runtime consumers use snapshots.

use core::str;

use crate::time::UtcOffset;

const MAGIC: &[u8; 4] = b"KCF1";
const FORMAT_MAJOR: u16 = 1;
const FORMAT_MINOR: u16 = 0;
const HEADER_LEN: usize = 24;
const RECORD_LEN: usize = 32;
const VALUE_LEN: usize = 24;
const KIND_UTF8: u8 = 1;
const KIND_I16: u8 = 2;
const KEY_LOCALE: u16 = 1;
const KEY_UTC_OFFSET: u16 = 2;
const KEY_SNTP_SERVER: u16 = 3;

pub const CONFIG_MAX_PUBLIC_SETTINGS: usize = 8;
pub const CONFIG_FORMAT_MAX_BYTES: usize = HEADER_LEN + CONFIG_MAX_PUBLIC_SETTINGS * RECORD_LEN;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigSlot {
    A,
    B,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    BufferTooSmall,
    BadMagic,
    UnsupportedVersion,
    InvalidLength,
    InvalidChecksum,
    InvalidGeneration,
    TooManySettings,
    DuplicateKey,
    InvalidRecord,
    InvalidUtf8,
    UnsupportedLocale,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Locale {
    #[default]
    EnUs,
    JaJp,
    /// Simulator/test-only expanded locale. Normal device UI does not list it.
    QpsPloc,
}

impl Locale {
    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::JaJp => "ja-JP",
            Self::QpsPloc => "qps-ploc",
        }
    }

    pub fn from_tag(tag: &str) -> Result<Self, ConfigError> {
        match tag {
            "en-US" => Ok(Self::EnUs),
            "ja-JP" => Ok(Self::JaJp),
            "qps-ploc" => Ok(Self::QpsPloc),
            _ => Err(ConfigError::UnsupportedLocale),
        }
    }
}

/// Curated advisory SNTP endpoints selectable by KotoConfig. Keeping this a
/// fixed enum bounds persistence, DNS input, UI text, and diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SntpServer {
    #[default]
    NtpPool,
    NictJapan,
    Cloudflare,
    Google,
}

impl SntpServer {
    pub const ALL: [Self; 4] = [
        Self::NtpPool,
        Self::NictJapan,
        Self::Cloudflare,
        Self::Google,
    ];

    pub const fn hostname(self) -> &'static str {
        match self {
            Self::NtpPool => "pool.ntp.org",
            Self::NictJapan => "ntp.nict.jp",
            Self::Cloudflare => "time.cloudflare.com",
            Self::Google => "time.google.com",
        }
    }

    pub fn from_hostname(hostname: &str) -> Result<Self, ConfigError> {
        Self::ALL
            .iter()
            .copied()
            .find(|server| server.hostname() == hostname)
            .ok_or(ConfigError::InvalidRecord)
    }

    pub const fn next(self) -> Self {
        match self {
            Self::NtpPool => Self::NictJapan,
            Self::NictJapan => Self::Cloudflare,
            Self::Cloudflare => Self::Google,
            Self::Google => Self::NtpPool,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PublicSetting {
    key: u16,
    kind: u8,
    len: u8,
    value: [u8; VALUE_LEN],
}

impl PublicSetting {
    const EMPTY: Self = Self {
        key: 0,
        kind: 0,
        len: 0,
        value: [0; VALUE_LEN],
    };

    fn locale(locale: Locale) -> Self {
        let mut setting = Self::EMPTY;
        setting.key = KEY_LOCALE;
        setting.kind = KIND_UTF8;
        let bytes = locale.tag().as_bytes();
        setting.len = bytes.len() as u8;
        setting.value[..bytes.len()].copy_from_slice(bytes);
        setting
    }

    fn utc_offset(offset: UtcOffset) -> Self {
        let mut setting = Self::EMPTY;
        setting.key = KEY_UTC_OFFSET;
        setting.kind = KIND_I16;
        setting.len = 2;
        setting.value[..2].copy_from_slice(&offset.minutes().to_le_bytes());
        setting
    }

    fn sntp_server(server: SntpServer) -> Self {
        let mut setting = Self::EMPTY;
        setting.key = KEY_SNTP_SERVER;
        setting.kind = KIND_UTF8;
        let bytes = server.hostname().as_bytes();
        setting.len = bytes.len() as u8;
        setting.value[..bytes.len()].copy_from_slice(bytes);
        setting
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.key == 0 || usize::from(self.len) > VALUE_LEN {
            return Err(ConfigError::InvalidRecord);
        }
        if self.kind == KIND_UTF8 {
            str::from_utf8(&self.value[..usize::from(self.len)])
                .map_err(|_| ConfigError::InvalidUtf8)?;
        }
        if self.key == KEY_LOCALE {
            if self.kind != KIND_UTF8 {
                return Err(ConfigError::InvalidRecord);
            }
            let tag = str::from_utf8(&self.value[..usize::from(self.len)])
                .map_err(|_| ConfigError::InvalidUtf8)?;
            Locale::from_tag(tag)?;
        }
        if self.key == KEY_UTC_OFFSET {
            if self.kind != KIND_I16 || self.len != 2 {
                return Err(ConfigError::InvalidRecord);
            }
            let minutes = i16::from_le_bytes([self.value[0], self.value[1]]);
            UtcOffset::from_minutes(minutes).ok_or(ConfigError::InvalidRecord)?;
        }
        if self.key == KEY_SNTP_SERVER {
            if self.kind != KIND_UTF8 {
                return Err(ConfigError::InvalidRecord);
            }
            let hostname = str::from_utf8(&self.value[..usize::from(self.len)])
                .map_err(|_| ConfigError::InvalidUtf8)?;
            SntpServer::from_hostname(hostname)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigSnapshot {
    pub locale: Locale,
    pub utc_offset: UtcOffset,
    pub sntp_server: SntpServer,
    pub generation: u32,
    pub locale_generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigService {
    settings: [PublicSetting; CONFIG_MAX_PUBLIC_SETTINGS],
    count: u8,
    generation: u32,
    locale_generation: u32,
}

pub fn newest_config_slot(
    a: Option<ConfigService>,
    b: Option<ConfigService>,
) -> Option<(ConfigSlot, ConfigService)> {
    match (a, b) {
        (Some(a), Some(b)) if config_generation_is_newer(b.generation(), a.generation()) => {
            Some((ConfigSlot::B, b))
        }
        (Some(a), Some(_)) => Some((ConfigSlot::A, a)),
        (Some(a), None) => Some((ConfigSlot::A, a)),
        (None, Some(b)) => Some((ConfigSlot::B, b)),
        (None, None) => None,
    }
}

pub fn config_write_slot(a: Option<ConfigService>, b: Option<ConfigService>) -> ConfigSlot {
    match newest_config_slot(a, b) {
        Some((ConfigSlot::A, _)) => ConfigSlot::B,
        Some((ConfigSlot::B, _)) => ConfigSlot::A,
        None => ConfigSlot::A,
    }
}

impl Default for ConfigService {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigService {
    pub const fn new() -> Self {
        let mut settings = [PublicSetting::EMPTY; CONFIG_MAX_PUBLIC_SETTINGS];
        settings[0] = PublicSetting {
            key: KEY_LOCALE,
            kind: KIND_UTF8,
            len: 5,
            value: [
                b'e', b'n', b'-', b'U', b'S', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0,
            ],
        };
        Self {
            settings,
            count: 1,
            generation: 1,
            locale_generation: 1,
        }
    }

    pub fn snapshot(&self) -> ConfigSnapshot {
        ConfigSnapshot {
            locale: self.locale(),
            utc_offset: self.utc_offset(),
            sntp_server: self.sntp_server(),
            generation: self.generation,
            locale_generation: self.locale_generation,
        }
    }

    pub fn generation(&self) -> u32 {
        self.generation
    }

    pub fn locale_generation(&self) -> u32 {
        self.locale_generation
    }

    pub fn locale(&self) -> Locale {
        self.settings[..usize::from(self.count)]
            .iter()
            .find(|setting| setting.key == KEY_LOCALE)
            .and_then(|setting| str::from_utf8(&setting.value[..usize::from(setting.len)]).ok())
            .and_then(|tag| Locale::from_tag(tag).ok())
            .unwrap_or(Locale::EnUs)
    }

    pub fn utc_offset(&self) -> UtcOffset {
        self.settings[..usize::from(self.count)]
            .iter()
            .find(|setting| setting.key == KEY_UTC_OFFSET)
            .filter(|setting| setting.kind == KIND_I16 && setting.len == 2)
            .and_then(|setting| {
                UtcOffset::from_minutes(i16::from_le_bytes([setting.value[0], setting.value[1]]))
            })
            .unwrap_or_default()
    }

    pub fn sntp_server(&self) -> SntpServer {
        self.settings[..usize::from(self.count)]
            .iter()
            .find(|setting| setting.key == KEY_SNTP_SERVER)
            .and_then(|setting| str::from_utf8(&setting.value[..usize::from(setting.len)]).ok())
            .and_then(|hostname| SntpServer::from_hostname(hostname).ok())
            .unwrap_or_default()
    }

    /// Changes locale and returns whether the public snapshot changed.
    pub fn set_locale(&mut self, locale: Locale) -> bool {
        if self.locale() == locale {
            return false;
        }
        let replacement = PublicSetting::locale(locale);
        if let Some(setting) = self.settings[..usize::from(self.count)]
            .iter_mut()
            .find(|setting| setting.key == KEY_LOCALE)
        {
            *setting = replacement;
        } else if usize::from(self.count) < CONFIG_MAX_PUBLIC_SETTINGS {
            self.settings[usize::from(self.count)] = replacement;
            self.count += 1;
        } else {
            // The locale record is mandatory for every valid service, so this
            // branch is defensive and cannot occur after construction/decode.
            return false;
        }
        self.generation = next_generation(self.generation);
        self.locale_generation = next_generation(self.locale_generation);
        true
    }

    /// Stores a fixed civil offset. Daylight-saving transitions are never
    /// inferred from locale and must be applied explicitly by the user.
    pub fn set_utc_offset(&mut self, offset: UtcOffset) -> bool {
        if self.utc_offset() == offset {
            return false;
        }
        let replacement = PublicSetting::utc_offset(offset);
        if let Some(setting) = self.settings[..usize::from(self.count)]
            .iter_mut()
            .find(|setting| setting.key == KEY_UTC_OFFSET)
        {
            *setting = replacement;
        } else if usize::from(self.count) < CONFIG_MAX_PUBLIC_SETTINGS {
            self.settings[usize::from(self.count)] = replacement;
            self.count += 1;
        } else {
            return false;
        }
        self.generation = next_generation(self.generation);
        true
    }

    pub fn set_sntp_server(&mut self, server: SntpServer) -> bool {
        if self.sntp_server() == server {
            return false;
        }
        let replacement = PublicSetting::sntp_server(server);
        if let Some(setting) = self.settings[..usize::from(self.count)]
            .iter_mut()
            .find(|setting| setting.key == KEY_SNTP_SERVER)
        {
            *setting = replacement;
        } else if usize::from(self.count) < CONFIG_MAX_PUBLIC_SETTINGS {
            self.settings[usize::from(self.count)] = replacement;
            self.count += 1;
        } else {
            return false;
        }
        self.generation = next_generation(self.generation);
        true
    }

    pub fn encoded_len(&self) -> usize {
        HEADER_LEN + usize::from(self.count) * RECORD_LEN
    }

    pub fn encode(&self, dst: &mut [u8]) -> Result<usize, ConfigError> {
        let total_len = self.encoded_len();
        if dst.len() < total_len {
            return Err(ConfigError::BufferTooSmall);
        }
        dst[..total_len].fill(0);
        dst[..4].copy_from_slice(MAGIC);
        put_u16(dst, 4, FORMAT_MAJOR);
        put_u16(dst, 6, FORMAT_MINOR);
        put_u16(dst, 8, total_len as u16);
        dst[10] = self.count;
        dst[11] = RECORD_LEN as u8;
        put_u32(dst, 12, self.generation);
        put_u32(dst, 20, self.locale_generation);

        for (index, setting) in self.settings[..usize::from(self.count)].iter().enumerate() {
            setting.validate()?;
            let offset = HEADER_LEN + index * RECORD_LEN;
            put_u16(dst, offset, setting.key);
            dst[offset + 2] = setting.kind;
            dst[offset + 3] = setting.len;
            dst[offset + 8..offset + 8 + VALUE_LEN].copy_from_slice(&setting.value);
        }
        let checksum = packet_checksum(&dst[..total_len]);
        put_u32(dst, 16, checksum);
        Ok(total_len)
    }

    pub fn decode(src: &[u8]) -> Result<Self, ConfigError> {
        if src.len() < HEADER_LEN {
            return Err(ConfigError::InvalidLength);
        }
        if &src[..4] != MAGIC {
            return Err(ConfigError::BadMagic);
        }
        if get_u16(src, 4) != FORMAT_MAJOR || get_u16(src, 6) != FORMAT_MINOR {
            return Err(ConfigError::UnsupportedVersion);
        }
        let total_len = usize::from(get_u16(src, 8));
        let count = usize::from(src[10]);
        if count == 0 || count > CONFIG_MAX_PUBLIC_SETTINGS {
            return Err(ConfigError::TooManySettings);
        }
        if src[11] != RECORD_LEN as u8 || total_len != HEADER_LEN + count * RECORD_LEN {
            return Err(ConfigError::InvalidLength);
        }
        if src.len() != total_len {
            return Err(ConfigError::InvalidLength);
        }
        let generation = get_u32(src, 12);
        let locale_generation = get_u32(src, 20);
        if generation == 0 || locale_generation == 0 {
            return Err(ConfigError::InvalidGeneration);
        }
        if get_u32(src, 16) != packet_checksum(src) {
            return Err(ConfigError::InvalidChecksum);
        }

        let mut settings = [PublicSetting::EMPTY; CONFIG_MAX_PUBLIC_SETTINGS];
        let mut locale_seen = false;
        for index in 0..count {
            let offset = HEADER_LEN + index * RECORD_LEN;
            if src[offset + 4..offset + 8].iter().any(|byte| *byte != 0) {
                return Err(ConfigError::InvalidRecord);
            }
            let key = get_u16(src, offset);
            if settings[..index].iter().any(|setting| setting.key == key) {
                return Err(ConfigError::DuplicateKey);
            }
            let mut setting = PublicSetting {
                key,
                kind: src[offset + 2],
                len: src[offset + 3],
                value: [0; VALUE_LEN],
            };
            setting
                .value
                .copy_from_slice(&src[offset + 8..offset + 8 + VALUE_LEN]);
            if usize::from(setting.len) > VALUE_LEN {
                return Err(ConfigError::InvalidRecord);
            }
            if setting.value[usize::from(setting.len)..]
                .iter()
                .any(|byte| *byte != 0)
            {
                return Err(ConfigError::InvalidRecord);
            }
            setting.validate()?;
            locale_seen |= key == KEY_LOCALE;
            settings[index] = setting;
        }
        if !locale_seen {
            return Err(ConfigError::InvalidRecord);
        }
        Ok(Self {
            settings,
            count: count as u8,
            generation,
            locale_generation,
        })
    }
}

/// Wrap-aware serial-number comparison for selecting the newest valid slot.
/// Generation zero is invalid and therefore never passed by storage adapters.
pub const fn config_generation_is_newer(candidate: u32, current: u32) -> bool {
    candidate != current && candidate.wrapping_sub(current) < 0x8000_0000
}

const fn next_generation(current: u32) -> u32 {
    let next = current.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

fn packet_checksum(bytes: &[u8]) -> u32 {
    let mut checksum = 0x811c_9dc5u32;
    for (index, byte) in bytes.iter().enumerate() {
        // The checksum field itself is interpreted as zero, both before and
        // after it has been written into the encoded packet.
        let value = if (16..20).contains(&index) { 0 } else { *byte };
        checksum = (checksum ^ u32::from(value)).wrapping_mul(0x0100_0193);
    }
    checksum
}

fn put_u16(dst: &mut [u8], offset: usize, value: u16) {
    dst[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut [u8], offset: usize, value: u32) {
    dst[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn get_u16(src: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([src[offset], src[offset + 1]])
}

fn get_u32(src: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        src[offset],
        src[offset + 1],
        src[offset + 2],
        src[offset + 3],
    ])
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigCapability(u32);

impl ConfigCapability {
    pub const LOCALE_CONFIG: Self = Self(1 << 0);
    pub const WIFI_RADIO: Self = Self(1 << 1);
    pub const WIFI_HAL: Self = Self(1 << 2);
    pub const NETWORK_SERVICE: Self = Self(1 << 3);
    pub const CREDENTIAL_PROVIDER: Self = Self(1 << 4);
    pub const WIFI_CONFIG: Self = Self(
        Self::WIFI_RADIO.0
            | Self::WIFI_HAL.0
            | Self::NETWORK_SERVICE.0
            | Self::CREDENTIAL_PROVIDER.0,
    );

    pub const fn bits(self) -> u32 {
        self.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, required: Self) -> bool {
        self.0 & required.0 == required.0
    }
}

/// The four runtime inputs to the composite `WIFI_CONFIG` capability
/// (KOTO-0224). Every input must be live at runtime; a board profile that merely
/// declares Wi-Fi, a board name suffix, or any single capability bit cannot
/// promote `WIFI_CONFIG` on its own.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WifiConfigInputs {
    /// The selected board profile declares a supported radio transport.
    pub supported_transport: bool,
    /// The Wi-Fi HAL initialized and can be quiesced by its lifecycle owner.
    pub hal_initialized: bool,
    /// The live `NetworkService`'s generation, if a service instance exists.
    pub network_service_generation: Option<u32>,
    /// The lifecycle owner's current generation. The `NetworkService` counts only
    /// when its generation equals this (a stale generation does not).
    pub lifecycle_generation: u32,
    /// The credential provider initialized its bounded secret namespace.
    pub credential_provider_ready: bool,
}

impl WifiConfigInputs {
    /// Maps each live input to its capability bit. `WIFI_CONFIG` is set only when
    /// all four hold; it is never inferred from any single bit.
    pub fn capability(&self) -> ConfigCapability {
        let mut capability = ConfigCapability(0);
        if self.supported_transport {
            capability = capability.union(ConfigCapability::WIFI_RADIO);
        }
        if self.hal_initialized {
            capability = capability.union(ConfigCapability::WIFI_HAL);
        }
        if self.network_service_live() {
            capability = capability.union(ConfigCapability::NETWORK_SERVICE);
        }
        if self.credential_provider_ready {
            capability = capability.union(ConfigCapability::CREDENTIAL_PROVIDER);
        }
        capability
    }

    /// Whether the composite `WIFI_CONFIG` capability is satisfied.
    pub fn wifi_config(&self) -> bool {
        self.capability().contains(ConfigCapability::WIFI_CONFIG)
    }

    /// A `NetworkService` is alive for the current lifecycle generation when its
    /// generation exists and matches the owner's current generation.
    fn network_service_live(&self) -> bool {
        self.network_service_generation == Some(self.lifecycle_generation)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigPageId {
    Language,
    Wifi,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigPageDescriptor {
    pub id: ConfigPageId,
    pub required: ConfigCapability,
    pub order: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConfigPageRegistry {
    pages: [ConfigPageDescriptor; 2],
    len: u8,
}

impl ConfigPageRegistry {
    pub fn from_capabilities(capabilities: ConfigCapability) -> Self {
        let language = ConfigPageDescriptor {
            id: ConfigPageId::Language,
            required: ConfigCapability::LOCALE_CONFIG,
            order: 0,
        };
        let wifi = ConfigPageDescriptor {
            id: ConfigPageId::Wifi,
            required: ConfigCapability::WIFI_CONFIG,
            order: 10,
        };
        let mut pages = [language, wifi];
        let mut len = 0;
        if capabilities.contains(language.required) {
            pages[len] = language;
            len += 1;
        }
        if capabilities.contains(wifi.required) {
            pages[len] = wifi;
            len += 1;
        }
        Self {
            pages,
            len: len as u8,
        }
    }

    pub fn pages(&self) -> &[ConfigPageDescriptor] {
        &self.pages[..usize::from(self.len)]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(service: &ConfigService) -> ([u8; CONFIG_FORMAT_MAX_BYTES], usize) {
        let mut bytes = [0; CONFIG_FORMAT_MAX_BYTES];
        let len = service.encode(&mut bytes).unwrap();
        (bytes, len)
    }

    #[test]
    fn defaults_to_english_and_generation_one() {
        assert_eq!(
            ConfigService::default().snapshot(),
            ConfigSnapshot {
                locale: Locale::EnUs,
                utc_offset: UtcOffset::default(),
                sntp_server: SntpServer::default(),
                generation: 1,
                locale_generation: 1,
            }
        );
    }

    #[test]
    fn locale_change_is_idempotent_and_round_trips() {
        let mut service = ConfigService::default();
        assert!(!service.set_locale(Locale::EnUs));
        assert!(service.set_locale(Locale::JaJp));
        assert_eq!(service.generation(), 2);
        assert_eq!(service.locale_generation(), 2);
        let (bytes, len) = encoded(&service);
        let decoded = ConfigService::decode(&bytes[..len]).unwrap();
        assert_eq!(decoded, service);
    }

    #[test]
    fn utc_offset_is_bounded_persisted_and_does_not_follow_locale() {
        let mut service = ConfigService::default();
        assert!(service.set_locale(Locale::JaJp));
        assert_eq!(service.utc_offset(), UtcOffset::default());
        let tokyo = UtcOffset::from_minutes(9 * 60).unwrap();
        assert!(service.set_utc_offset(tokyo));
        assert!(!service.set_utc_offset(tokyo));
        let (bytes, len) = encoded(&service);
        let decoded = ConfigService::decode(&bytes[..len]).unwrap();
        assert_eq!(decoded.utc_offset(), tokyo);
        assert_eq!(decoded.snapshot().utc_offset, tokyo);
    }

    #[test]
    fn sntp_server_selection_is_curated_and_persisted() {
        let mut service = ConfigService::default();
        assert_eq!(service.sntp_server(), SntpServer::NtpPool);
        assert!(service.set_sntp_server(SntpServer::NictJapan));
        assert!(!service.set_sntp_server(SntpServer::NictJapan));
        let (bytes, len) = encoded(&service);
        let decoded = ConfigService::decode(&bytes[..len]).unwrap();
        assert_eq!(decoded.sntp_server(), SntpServer::NictJapan);
        assert_eq!(decoded.snapshot().sntp_server, SntpServer::NictJapan);
        assert!(SntpServer::from_hostname("arbitrary.example").is_err());
    }

    #[test]
    fn generation_wrap_skips_zero() {
        let mut service = ConfigService {
            generation: u32::MAX,
            locale_generation: u32::MAX,
            ..ConfigService::default()
        };
        assert!(service.set_locale(Locale::JaJp));
        assert_eq!(service.generation(), 1);
        assert_eq!(service.locale_generation(), 1);
    }

    #[test]
    fn checksum_and_incomplete_records_are_rejected() {
        let service = ConfigService::default();
        let (mut bytes, len) = encoded(&service);
        bytes[len - 1] ^= 1;
        assert_eq!(
            ConfigService::decode(&bytes[..len]),
            Err(ConfigError::InvalidChecksum)
        );
        assert_eq!(
            ConfigService::decode(&bytes[..len - 1]),
            Err(ConfigError::InvalidLength)
        );
    }

    #[test]
    fn unknown_locale_is_rejected_for_adapter_fallback() {
        let service = ConfigService::default();
        let (mut bytes, len) = encoded(&service);
        bytes[HEADER_LEN + 3] = 5;
        bytes[HEADER_LEN + 8..HEADER_LEN + 13].copy_from_slice(b"xx-XX");
        let checksum = packet_checksum(&bytes[..len]);
        put_u32(&mut bytes, 16, checksum);
        assert_eq!(
            ConfigService::decode(&bytes[..len]),
            Err(ConfigError::UnsupportedLocale)
        );
    }

    #[test]
    fn compatible_unknown_key_is_preserved_byte_for_byte() {
        let service = ConfigService::default();
        let (mut bytes, original_len) = encoded(&service);
        let new_len = original_len + RECORD_LEN;
        put_u16(&mut bytes, 8, new_len as u16);
        bytes[10] = 2;
        put_u16(&mut bytes, original_len, 99);
        bytes[original_len + 2] = 0x80;
        bytes[original_len + 3] = 3;
        bytes[original_len + 8..original_len + 11].copy_from_slice(&[1, 2, 3]);
        let checksum = packet_checksum(&bytes[..new_len]);
        put_u32(&mut bytes, 16, checksum);

        let decoded = ConfigService::decode(&bytes[..new_len]).unwrap();
        let (round_trip, round_trip_len) = encoded(&decoded);
        assert_eq!(round_trip_len, new_len);
        assert_eq!(&round_trip[..new_len], &bytes[..new_len]);
    }

    #[test]
    fn oversized_record_length_is_rejected_without_panicking() {
        let service = ConfigService::default();
        let (mut bytes, len) = encoded(&service);
        bytes[HEADER_LEN + 3] = (VALUE_LEN + 1) as u8;
        let checksum = packet_checksum(&bytes[..len]);
        put_u32(&mut bytes, 16, checksum);
        assert_eq!(
            ConfigService::decode(&bytes[..len]),
            Err(ConfigError::InvalidRecord)
        );
    }

    #[test]
    fn wifi_page_requires_every_composite_capability() {
        let locale_only = ConfigPageRegistry::from_capabilities(ConfigCapability::LOCALE_CONFIG);
        assert_eq!(locale_only.pages().len(), 1);
        assert_eq!(locale_only.pages()[0].id, ConfigPageId::Language);

        let missing_credentials = ConfigCapability::LOCALE_CONFIG
            .union(ConfigCapability::WIFI_RADIO)
            .union(ConfigCapability::WIFI_HAL)
            .union(ConfigCapability::NETWORK_SERVICE);
        assert_eq!(
            ConfigPageRegistry::from_capabilities(missing_credentials)
                .pages()
                .len(),
            1
        );

        let all = missing_credentials.union(ConfigCapability::CREDENTIAL_PROVIDER);
        let pages = ConfigPageRegistry::from_capabilities(all);
        assert_eq!(pages.pages().len(), 2);
        assert_eq!(pages.pages()[1].id, ConfigPageId::Wifi);
    }

    fn all_wifi_inputs_live() -> WifiConfigInputs {
        WifiConfigInputs {
            supported_transport: true,
            hal_initialized: true,
            network_service_generation: Some(7),
            lifecycle_generation: 7,
            credential_provider_ready: true,
        }
    }

    #[test]
    fn wifi_config_requires_all_four_runtime_inputs() {
        let inputs = all_wifi_inputs_live();
        assert!(inputs.wifi_config());
        assert!(inputs.capability().contains(ConfigCapability::WIFI_CONFIG));

        // Removing any single input drops the composite.
        let mut without_transport = all_wifi_inputs_live();
        without_transport.supported_transport = false;
        assert!(!without_transport.wifi_config());

        let mut without_hal = all_wifi_inputs_live();
        without_hal.hal_initialized = false;
        assert!(!without_hal.wifi_config());

        let mut without_service = all_wifi_inputs_live();
        without_service.network_service_generation = None;
        assert!(!without_service.wifi_config());

        let mut without_credentials = all_wifi_inputs_live();
        without_credentials.credential_provider_ready = false;
        assert!(!without_credentials.wifi_config());
    }

    #[test]
    fn wifi_config_rejects_a_single_bit_or_board_declaration() {
        // A board that only declares a supported transport (a board-name/bit
        // "WIFI" signal) cannot promote WIFI_CONFIG alone.
        let transport_only = WifiConfigInputs {
            supported_transport: true,
            hal_initialized: false,
            network_service_generation: None,
            lifecycle_generation: 1,
            credential_provider_ready: false,
        };
        assert!(!transport_only.wifi_config());
        assert_eq!(transport_only.capability(), ConfigCapability::WIFI_RADIO);
    }

    #[test]
    fn wifi_config_rejects_a_stale_network_service_generation() {
        let mut stale = all_wifi_inputs_live();
        // The service belongs to a prior lifecycle generation.
        stale.network_service_generation = Some(6);
        stale.lifecycle_generation = 7;
        assert!(!stale.wifi_config());
        assert!(!stale
            .capability()
            .contains(ConfigCapability::NETWORK_SERVICE));
    }

    #[test]
    fn wifi_config_inputs_feed_the_page_registry() {
        let inputs = all_wifi_inputs_live();
        let capability = inputs.capability().union(ConfigCapability::LOCALE_CONFIG);
        let pages = ConfigPageRegistry::from_capabilities(capability);
        assert_eq!(pages.pages().len(), 2);
        assert_eq!(pages.pages()[1].id, ConfigPageId::Wifi);
    }

    #[test]
    fn pseudolocale_is_serializable_but_not_a_default() {
        let mut service = ConfigService::default();
        assert!(service.set_locale(Locale::QpsPloc));
        let (bytes, len) = encoded(&service);
        assert_eq!(
            ConfigService::decode(&bytes[..len]).unwrap().locale(),
            Locale::QpsPloc
        );
    }

    #[test]
    fn slot_policy_tolerates_missing_corrupt_and_stale_records() {
        let first = ConfigService::default();
        let mut second = first;
        assert!(second.set_locale(Locale::JaJp));

        assert_eq!(newest_config_slot(None, None), None);
        assert_eq!(
            newest_config_slot(Some(first), None),
            Some((ConfigSlot::A, first))
        );
        assert_eq!(
            newest_config_slot(None, Some(second)),
            Some((ConfigSlot::B, second))
        );
        assert_eq!(
            newest_config_slot(Some(first), Some(second)),
            Some((ConfigSlot::B, second))
        );
        assert_eq!(config_write_slot(None, None), ConfigSlot::A);
        assert_eq!(config_write_slot(Some(first), None), ConfigSlot::B);
        assert_eq!(config_write_slot(None, Some(second)), ConfigSlot::A);
        assert_eq!(config_write_slot(Some(first), Some(second)), ConfigSlot::A);

        let (mut corrupt, len) = encoded(&second);
        corrupt[len - 1] ^= 1;
        let failed = ConfigService::decode(&corrupt[..len]).ok();
        assert_eq!(
            newest_config_slot(Some(first), failed),
            Some((ConfigSlot::A, first))
        );
        assert_eq!(config_write_slot(Some(first), failed), ConfigSlot::B);
    }
}
