//! Bounded application MQTT subscribe service (KOTO-0249).
//!
//! This module is deliberately transport-neutral and `no_std`. It freezes a
//! fixed-capacity MQTT 3.1.1 *subscribe-only, QoS 0* profile above the
//! firmware-owned `NetworkService`: broker origins, exact topic filters, the
//! session lifecycle, a bounded inbound message queue, and a portable incoming
//! packet decoder. Applications receive generation-tagged session IDs and
//! copied bytes only; the OS-owned backend retains every socket and TLS object.
//! A simulator fake and the firmware network task therefore share the same
//! permission, framing, and lifecycle policy.
//!
//! Non-goals for this profile: publishing, QoS 1/2, MQTT 5, WebSocket
//! transport, wildcard topic filters, multiple simultaneous brokers, and
//! background delivery while an application is inactive.

use crate::fetch::AppContext;

/// MQTT protocol level byte for 3.1.1.
pub const MQTT_PROTOCOL_LEVEL: u8 = 4;
/// Canonical broker origins an application may declare in its manifest.
pub const MAX_MQTT_BROKERS: usize = 2;
/// Exact topic filters an application may declare in its manifest.
pub const MAX_MQTT_TOPIC_FILTERS: usize = 8;
/// Maximum topic-filter and topic-name length in UTF-8 bytes.
pub const MAX_MQTT_TOPIC_BYTES: usize = 128;
/// Maximum payload bytes copied out of a single delivered message.
pub const MAX_MQTT_PAYLOAD_BYTES: usize = 192;
/// Total incoming MQTT control-packet size limit, enforced before any copy.
pub const MAX_MQTT_PACKET_BYTES: usize = 256;
/// Depth of the OS-owned inbound message queue (per active session).
pub const MAX_MQTT_MESSAGE_QUEUE: usize = 8;
/// At most one active broker session across all applications.
pub const MAX_GLOBAL_MQTT_SESSIONS: usize = 1;
/// Maximum broker hostname length in bytes.
pub const MAX_MQTT_HOSTNAME_BYTES: usize = 253;
/// Keepalive interval negotiated in CONNECT, in seconds.
pub const MQTT_KEEPALIVE_SECS: u16 = 60;
/// Deadline for the connect + CONNACK handshake, in milliseconds.
pub const MQTT_CONNECT_DEADLINE_MS: u32 = 15_000;
/// Lower bound of the reconnect backoff, in milliseconds.
pub const MQTT_RECONNECT_MIN_MS: u32 = 1_000;
/// Upper bound of the reconnect backoff, in milliseconds.
pub const MQTT_RECONNECT_MAX_MS: u32 = 30_000;

/// Stable ABI codes for the app-visible MQTT service (Host ABI minor 23,
/// KOTO-0249). The SDK constants (`koto-compiler`) and every host implementation
/// (`koto-sim`, firmware) share these so the numbers cannot drift from the
/// `mqtt_poll` / `mqtt_read` contract.
pub mod app_mqtt {
    /// `mqtt_poll` lifecycle states.
    pub const STATE_CONNECTING: i32 = 0;
    pub const STATE_CONNECTED: i32 = 1;
    pub const STATE_MESSAGE: i32 = 2;
    pub const STATE_DISCONNECTED: i32 = 3;
    pub const STATE_FAILED: i32 = 4;

    /// `mqtt_read` delivery results.
    pub const READ_NONE: i32 = 0;
    pub const READ_MESSAGE: i32 = 1;
    pub const READ_RETAINED: i32 = 2;
}

// ---------------------------------------------------------------------------
// Broker origin
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MqttScheme {
    /// Plain MQTT over TCP. Development-only; disabled in release profiles.
    Mqtt = 0,
    /// MQTT over TLS.
    Mqtts = 1,
}

impl MqttScheme {
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Mqtt => 1883,
            Self::Mqtts => 8883,
        }
    }

    /// Release profiles carry authenticated brokers only; plain MQTT is a
    /// development affordance and never linked into a release build.
    pub const fn release_allowed(self) -> bool {
        matches!(self, Self::Mqtts)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerError {
    Malformed,
    UnsupportedScheme,
    HostnameTooLong,
    InvalidHostname,
    InvalidPort,
    UserInfo,
    Wildcard,
    NonCanonical,
}

/// A canonical `(scheme, hostname, port)` broker origin. Stored inline with no
/// heap and no retained pointer into caller input.
#[derive(Clone, Copy)]
pub struct MqttOrigin {
    scheme: MqttScheme,
    port: u16,
    host_len: u8,
    host: [u8; MAX_MQTT_HOSTNAME_BYTES],
}

impl core::fmt::Debug for MqttOrigin {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MqttOrigin")
            .field("scheme", &self.scheme)
            .field("host", &self.hostname())
            .field("port", &self.port)
            .finish()
    }
}

impl PartialEq for MqttOrigin {
    fn eq(&self, other: &Self) -> bool {
        self.scheme == other.scheme
            && self.port == other.port
            && self.host_len == other.host_len
            && self.host[..self.host_len as usize] == other.host[..other.host_len as usize]
    }
}

impl Eq for MqttOrigin {}

impl MqttOrigin {
    /// Parse a canonical broker origin such as `mqtts://broker.example.com` or
    /// `mqtt://192.0.2.10:1884`. The default port must be omitted (canonical
    /// form), wildcards and user-info are rejected, and the hostname is lower
    /// ASCII only.
    pub fn parse(text: &str) -> Result<Self, BrokerError> {
        let (scheme, rest) = if let Some(rest) = text.strip_prefix("mqtts://") {
            (MqttScheme::Mqtts, rest)
        } else if let Some(rest) = text.strip_prefix("mqtt://") {
            (MqttScheme::Mqtt, rest)
        } else if text.contains("://") {
            return Err(BrokerError::UnsupportedScheme);
        } else {
            return Err(BrokerError::Malformed);
        };

        if rest.is_empty() {
            return Err(BrokerError::Malformed);
        }
        // No path, query, fragment, or user-info in a broker authority.
        if rest.contains('/') || rest.contains('?') || rest.contains('#') {
            return Err(BrokerError::Malformed);
        }
        if rest.contains('@') {
            return Err(BrokerError::UserInfo);
        }
        if rest.contains('*') {
            return Err(BrokerError::Wildcard);
        }

        let (host, port) = match rest.rsplit_once(':') {
            Some((host, port_text)) => {
                let port: u16 = port_text.parse().map_err(|_| BrokerError::InvalidPort)?;
                if port == 0 {
                    return Err(BrokerError::InvalidPort);
                }
                if port == scheme.default_port() {
                    // Canonical form omits the default port.
                    return Err(BrokerError::NonCanonical);
                }
                (host, port)
            }
            None => (rest, scheme.default_port()),
        };

        if host.is_empty() {
            return Err(BrokerError::InvalidHostname);
        }
        if host.len() > MAX_MQTT_HOSTNAME_BYTES {
            return Err(BrokerError::HostnameTooLong);
        }
        if !is_canonical_hostname(host) {
            return Err(BrokerError::InvalidHostname);
        }

        let mut buffer = [0u8; MAX_MQTT_HOSTNAME_BYTES];
        buffer[..host.len()].copy_from_slice(host.as_bytes());
        Ok(Self {
            scheme,
            port,
            host_len: host.len() as u8,
            host: buffer,
        })
    }

    pub fn scheme(&self) -> MqttScheme {
        self.scheme
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn hostname(&self) -> &str {
        // Invariant: only canonical lower-ASCII bytes are stored.
        core::str::from_utf8(&self.host[..self.host_len as usize]).unwrap_or("")
    }
}

/// Accept lower-case letters, digits, `-`, and `.` label separators. This is
/// the same conservative canonical-hostname shape the Fetch origin uses; upper
/// case, empty labels, and leading/trailing dots are non-canonical.
fn is_canonical_hostname(host: &str) -> bool {
    if host.starts_with('.') || host.ends_with('.') || host.contains("..") {
        return false;
    }
    host.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
    }) && host
        .split('.')
        .all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-'))
}

// ---------------------------------------------------------------------------
// Topic filters (exact, no wildcards in this profile)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopicError {
    Empty,
    TooLong,
    InvalidUtf8,
    Wildcard,
    ControlCharacter,
}

/// A single exact topic filter. Wildcards (`+`, `#`) are rejected: this profile
/// freezes exact filters only, so a broker cannot broaden a subscription.
#[derive(Clone, Copy)]
pub struct TopicFilter {
    len: u8,
    bytes: [u8; MAX_MQTT_TOPIC_BYTES],
}

impl core::fmt::Debug for TopicFilter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("TopicFilter").field(&self.as_str()).finish()
    }
}

impl PartialEq for TopicFilter {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len
            && self.bytes[..self.len as usize] == other.bytes[..other.len as usize]
    }
}

impl Eq for TopicFilter {}

impl TopicFilter {
    pub fn parse(text: &str) -> Result<Self, TopicError> {
        Self::from_bytes(text.as_bytes())
    }

    /// Validate untrusted bytes (from a manifest or a PUBLISH frame) as an exact
    /// topic. Applies the MQTT rule set this profile supports: non-empty, length
    /// bounded, valid UTF-8, no wildcards, no control characters (including the
    /// forbidden `U+0000`).
    pub fn from_bytes(raw: &[u8]) -> Result<Self, TopicError> {
        if raw.is_empty() {
            return Err(TopicError::Empty);
        }
        if raw.len() > MAX_MQTT_TOPIC_BYTES {
            return Err(TopicError::TooLong);
        }
        let text = core::str::from_utf8(raw).map_err(|_| TopicError::InvalidUtf8)?;
        for ch in text.chars() {
            match ch {
                '+' | '#' => return Err(TopicError::Wildcard),
                '\0' => return Err(TopicError::ControlCharacter),
                c if (c as u32) < 0x20 || (c as u32) == 0x7f => {
                    return Err(TopicError::ControlCharacter)
                }
                _ => {}
            }
        }
        let mut bytes = [0u8; MAX_MQTT_TOPIC_BYTES];
        bytes[..raw.len()].copy_from_slice(raw);
        Ok(Self {
            len: raw.len() as u8,
            bytes,
        })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len as usize]
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(self.as_bytes()).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopicSetError {
    Full,
    Duplicate,
}

/// A fixed-capacity, duplicate-free set of declared topic filters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TopicFilterSet {
    filters: [Option<TopicFilter>; MAX_MQTT_TOPIC_FILTERS],
    len: usize,
}

impl Default for TopicFilterSet {
    fn default() -> Self {
        Self::empty()
    }
}

impl TopicFilterSet {
    pub const fn empty() -> Self {
        Self {
            filters: [None; MAX_MQTT_TOPIC_FILTERS],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, filter: TopicFilter) -> Result<(), TopicSetError> {
        if self.filters[..self.len].contains(&Some(filter)) {
            return Err(TopicSetError::Duplicate);
        }
        if self.len == MAX_MQTT_TOPIC_FILTERS {
            return Err(TopicSetError::Full);
        }
        self.filters[self.len] = Some(filter);
        self.len += 1;
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<TopicFilter> {
        self.filters.get(index).copied().flatten()
    }

    /// Index of an exact-matching filter, if the topic was declared.
    pub fn position(&self, topic: &[u8]) -> Option<usize> {
        self.filters[..self.len]
            .iter()
            .position(|entry| entry.is_some_and(|filter| filter.as_bytes() == topic))
    }

    pub fn contains_topic(&self, topic: &[u8]) -> bool {
        self.position(topic).is_some()
    }
}

// ---------------------------------------------------------------------------
// Broker allowlist
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrokerListError {
    Full,
    Duplicate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrokerAllowlist {
    brokers: [Option<MqttOrigin>; MAX_MQTT_BROKERS],
    len: usize,
}

impl Default for BrokerAllowlist {
    fn default() -> Self {
        Self::empty()
    }
}

impl BrokerAllowlist {
    pub const fn empty() -> Self {
        Self {
            brokers: [None; MAX_MQTT_BROKERS],
            len: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn push(&mut self, origin: MqttOrigin) -> Result<(), BrokerListError> {
        if self.brokers[..self.len].contains(&Some(origin)) {
            return Err(BrokerListError::Duplicate);
        }
        if self.len == MAX_MQTT_BROKERS {
            return Err(BrokerListError::Full);
        }
        self.brokers[self.len] = Some(origin);
        self.len += 1;
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<MqttOrigin> {
        self.brokers.get(index).copied().flatten()
    }

    pub fn position(&self, origin: &MqttOrigin) -> Option<usize> {
        self.brokers[..self.len]
            .iter()
            .position(|entry| entry.as_ref() == Some(origin))
    }
}

// ---------------------------------------------------------------------------
// Manifest permission
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestMqttError {
    InvalidJson,
    InvalidShape,
    InvalidBroker,
    InvalidTopic,
    UnsupportedVersion,
}

/// The parsed `permissions.mqtt` declaration. Default-denied: absent members
/// yield empty broker and topic sets.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestMqttPermission {
    pub brokers: BrokerAllowlist,
    pub topics: TopicFilterSet,
}

impl ManifestMqttPermission {
    pub const fn empty() -> Self {
        Self {
            brokers: BrokerAllowlist::empty(),
            topics: TopicFilterSet::empty(),
        }
    }

    pub fn is_declared(&self) -> bool {
        !self.brokers.is_empty()
    }
}

/// Parse the root `permissions.mqtt` member of a KPA manifest without
/// allocation. Unknown members are structurally skipped with a fixed nesting
/// limit, so a nested attacker-controlled `mqtt` key cannot be mistaken for the
/// root permission declaration. Only manifest schema version 2 is supported.
pub fn parse_manifest_mqtt_permission(
    manifest: &str,
    version: u32,
) -> Result<ManifestMqttPermission, ManifestMqttError> {
    if version != 2 {
        return Err(ManifestMqttError::UnsupportedVersion);
    }
    let mut cursor = ManifestCursor::new(manifest.as_bytes());
    cursor.ws();
    cursor.byte(b'{')?;
    let mut permission = ManifestMqttPermission::empty();
    let mut saw_permissions = false;
    cursor.ws();
    if cursor.take(b'}') {
        cursor.finish()?;
        return Ok(permission);
    }
    loop {
        let key = cursor.string()?;
        cursor.ws();
        cursor.byte(b':')?;
        cursor.ws();
        if key == b"permissions" {
            if saw_permissions {
                return Err(ManifestMqttError::InvalidShape);
            }
            saw_permissions = true;
            permission = cursor.permissions()?;
        } else {
            cursor.value(1)?;
        }
        cursor.ws();
        if cursor.take(b'}') {
            break;
        }
        cursor.byte(b',')?;
        cursor.ws();
    }
    cursor.finish()?;
    Ok(permission)
}

const MANIFEST_MQTT_MAX_DEPTH: u8 = 8;

struct ManifestCursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> ManifestCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, at: 0 }
    }

    fn ws(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.at += 1;
        }
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.bytes.get(self.at) == Some(&expected) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn byte(&mut self, expected: u8) -> Result<(), ManifestMqttError> {
        self.take(expected)
            .then_some(())
            .ok_or(ManifestMqttError::InvalidJson)
    }

    fn finish(&mut self) -> Result<(), ManifestMqttError> {
        self.ws();
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(ManifestMqttError::InvalidJson)
        }
    }

    /// Read a JSON string, rejecting escapes (manifest broker/topic strings are
    /// plain ASCII/UTF-8 with no need for escape sequences), and return the raw
    /// bytes between the quotes.
    fn string(&mut self) -> Result<&'a [u8], ManifestMqttError> {
        self.byte(b'"')?;
        let start = self.at;
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                b'"' => {
                    let raw = &self.bytes[start..self.at];
                    self.at += 1;
                    return Ok(raw);
                }
                b'\\' => return Err(ManifestMqttError::InvalidJson),
                0x00..=0x1f => return Err(ManifestMqttError::InvalidJson),
                _ => self.at += 1,
            }
        }
        Err(ManifestMqttError::InvalidJson)
    }

    fn permissions(&mut self) -> Result<ManifestMqttPermission, ManifestMqttError> {
        self.byte(b'{')?;
        let mut result = ManifestMqttPermission::empty();
        let mut saw_mqtt = false;
        self.ws();
        if self.take(b'}') {
            return Ok(result);
        }
        loop {
            let key = self.string()?;
            self.ws();
            self.byte(b':')?;
            self.ws();
            if key == b"mqtt" {
                if saw_mqtt {
                    return Err(ManifestMqttError::InvalidShape);
                }
                saw_mqtt = true;
                result = self.mqtt_object()?;
            } else {
                self.value(2)?;
            }
            self.ws();
            if self.take(b'}') {
                return Ok(result);
            }
            self.byte(b',')?;
            self.ws();
        }
    }

    fn mqtt_object(&mut self) -> Result<ManifestMqttPermission, ManifestMqttError> {
        self.byte(b'{')?;
        let mut result = ManifestMqttPermission::empty();
        let mut saw_brokers = false;
        let mut saw_topics = false;
        self.ws();
        if self.take(b'}') {
            return Err(ManifestMqttError::InvalidShape);
        }
        loop {
            let key = self.string()?;
            self.ws();
            self.byte(b':')?;
            self.ws();
            match key {
                b"brokers" if !saw_brokers => {
                    saw_brokers = true;
                    result.brokers = self.brokers()?;
                }
                b"topics" if !saw_topics => {
                    saw_topics = true;
                    result.topics = self.topics()?;
                }
                _ => return Err(ManifestMqttError::InvalidShape),
            }
            self.ws();
            if self.take(b'}') {
                break;
            }
            self.byte(b',')?;
            self.ws();
        }
        // A declaration must name at least one broker and one topic to connect.
        if !saw_brokers || !saw_topics || result.brokers.is_empty() || result.topics.is_empty() {
            return Err(ManifestMqttError::InvalidShape);
        }
        Ok(result)
    }

    fn brokers(&mut self) -> Result<BrokerAllowlist, ManifestMqttError> {
        self.byte(b'[')?;
        let mut list = BrokerAllowlist::empty();
        self.ws();
        if self.take(b']') {
            return Err(ManifestMqttError::InvalidShape);
        }
        loop {
            let raw = self.string()?;
            let text = core::str::from_utf8(raw).map_err(|_| ManifestMqttError::InvalidBroker)?;
            let origin = MqttOrigin::parse(text).map_err(|_| ManifestMqttError::InvalidBroker)?;
            list.push(origin)
                .map_err(|_| ManifestMqttError::InvalidBroker)?;
            self.ws();
            if self.take(b']') {
                return Ok(list);
            }
            self.byte(b',')?;
            self.ws();
        }
    }

    fn topics(&mut self) -> Result<TopicFilterSet, ManifestMqttError> {
        self.byte(b'[')?;
        let mut set = TopicFilterSet::empty();
        self.ws();
        if self.take(b']') {
            return Err(ManifestMqttError::InvalidShape);
        }
        loop {
            let raw = self.string()?;
            let filter =
                TopicFilter::from_bytes(raw).map_err(|_| ManifestMqttError::InvalidTopic)?;
            set.push(filter)
                .map_err(|_| ManifestMqttError::InvalidTopic)?;
            self.ws();
            if self.take(b']') {
                return Ok(set);
            }
            self.byte(b',')?;
            self.ws();
        }
    }

    /// Structurally skip an unknown JSON value with a bounded nesting depth.
    fn value(&mut self, depth: u8) -> Result<(), ManifestMqttError> {
        if depth > MANIFEST_MQTT_MAX_DEPTH {
            return Err(ManifestMqttError::InvalidShape);
        }
        self.ws();
        match self.bytes.get(self.at) {
            Some(b'"') => {
                self.string()?;
                Ok(())
            }
            Some(b'{') => {
                self.at += 1;
                self.ws();
                if self.take(b'}') {
                    return Ok(());
                }
                loop {
                    self.string()?;
                    self.ws();
                    self.byte(b':')?;
                    self.value(depth + 1)?;
                    self.ws();
                    if self.take(b'}') {
                        return Ok(());
                    }
                    self.byte(b',')?;
                    self.ws();
                }
            }
            Some(b'[') => {
                self.at += 1;
                self.ws();
                if self.take(b']') {
                    return Ok(());
                }
                loop {
                    self.value(depth + 1)?;
                    self.ws();
                    if self.take(b']') {
                        return Ok(());
                    }
                    self.byte(b',')?;
                    self.ws();
                }
            }
            Some(_) => {
                // number, true, false, null: consume until a structural byte.
                let start = self.at;
                while let Some(&byte) = self.bytes.get(self.at) {
                    if matches!(byte, b',' | b'}' | b']') || byte.is_ascii_whitespace() {
                        break;
                    }
                    self.at += 1;
                }
                (self.at > start)
                    .then_some(())
                    .ok_or(ManifestMqttError::InvalidJson)
            }
            None => Err(ManifestMqttError::InvalidJson),
        }
    }
}

// ---------------------------------------------------------------------------
// Bounded inbound message queue
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct QueuedMessage {
    topic_len: u16,
    payload_len: u16,
    retained: bool,
    topic: [u8; MAX_MQTT_TOPIC_BYTES],
    payload: [u8; MAX_MQTT_PAYLOAD_BYTES],
}

impl QueuedMessage {
    const fn zeroed() -> Self {
        Self {
            topic_len: 0,
            payload_len: 0,
            retained: false,
            topic: [0; MAX_MQTT_TOPIC_BYTES],
            payload: [0; MAX_MQTT_PAYLOAD_BYTES],
        }
    }

    fn zeroize(&mut self) {
        *self = Self::zeroed();
    }
}

/// Metadata describing a delivered message copied into caller buffers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqttMessage {
    pub topic_len: u16,
    pub payload_len: u16,
    pub retained: bool,
}

/// A fixed-depth ring of complete messages. Overflow policy is *drop-oldest*:
/// when full, the oldest unread message is evicted so the freshest telemetry is
/// retained, and a saturating counter records the loss. Memory never grows with
/// message rate or payload size.
#[derive(Debug)]
pub struct MqttMessageQueue {
    slots: [QueuedMessage; MAX_MQTT_MESSAGE_QUEUE],
    head: usize,
    len: usize,
    dropped: u32,
}

impl Default for MqttMessageQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttMessageQueue {
    pub const fn new() -> Self {
        Self {
            slots: [QueuedMessage::zeroed(); MAX_MQTT_MESSAGE_QUEUE],
            head: 0,
            len: 0,
            dropped: 0,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn dropped(&self) -> u32 {
        self.dropped
    }

    /// Enqueue a complete message. `topic` and `payload` must already be within
    /// the frozen maxima (the decoder guarantees this before calling). When the
    /// ring is full the oldest message is evicted first.
    pub fn push(&mut self, topic: &[u8], payload: &[u8], retained: bool) {
        debug_assert!(topic.len() <= MAX_MQTT_TOPIC_BYTES);
        debug_assert!(payload.len() <= MAX_MQTT_PAYLOAD_BYTES);
        if self.len == MAX_MQTT_MESSAGE_QUEUE {
            // Drop the oldest to admit the newest.
            self.slots[self.head].zeroize();
            self.head = (self.head + 1) % MAX_MQTT_MESSAGE_QUEUE;
            self.len -= 1;
            self.dropped = self.dropped.saturating_add(1);
        }
        let tail = (self.head + self.len) % MAX_MQTT_MESSAGE_QUEUE;
        let slot = &mut self.slots[tail];
        slot.topic_len = topic.len() as u16;
        slot.payload_len = payload.len() as u16;
        slot.retained = retained;
        slot.topic[..topic.len()].copy_from_slice(topic);
        slot.payload[..payload.len()].copy_from_slice(payload);
        self.len += 1;
    }

    /// Peek at the full lengths of the oldest queued message, without removing
    /// it. Used to decide whether a caller buffer is large enough.
    pub fn front_lengths(&self) -> Option<(usize, usize)> {
        (self.len > 0).then(|| {
            let slot = &self.slots[self.head];
            (slot.topic_len as usize, slot.payload_len as usize)
        })
    }

    /// Pop the oldest message, copying it into caller buffers. Returns `None`
    /// when empty. On a buffer too small for either field, the message is still
    /// consumed and `Err(MqttError::MessageTooLarge)` is returned — a truncated
    /// message is never reported as a complete delivery.
    pub fn pop(
        &mut self,
        topic: &mut [u8],
        payload: &mut [u8],
    ) -> Result<Option<MqttMessage>, MqttError> {
        if self.len == 0 {
            return Ok(None);
        }
        let index = self.head;
        let (topic_len, payload_len, retained) = {
            let slot = &self.slots[index];
            (
                slot.topic_len as usize,
                slot.payload_len as usize,
                slot.retained,
            )
        };
        let fits = topic_len <= topic.len() && payload_len <= payload.len();
        if fits {
            topic[..topic_len].copy_from_slice(&self.slots[index].topic[..topic_len]);
            payload[..payload_len].copy_from_slice(&self.slots[index].payload[..payload_len]);
        }
        self.slots[index].zeroize();
        self.head = (self.head + 1) % MAX_MQTT_MESSAGE_QUEUE;
        self.len -= 1;
        if !fits {
            return Err(MqttError::MessageTooLarge);
        }
        Ok(Some(MqttMessage {
            topic_len: topic_len as u16,
            payload_len: payload_len as u16,
            retained,
        }))
    }

    /// Zeroize and empty the queue on teardown / capability loss.
    pub fn clear(&mut self) {
        for slot in self.slots.iter_mut() {
            slot.zeroize();
        }
        self.head = 0;
        self.len = 0;
    }
}

// ---------------------------------------------------------------------------
// Incoming packet decoder (MQTT 3.1.1, subscribe path)
// ---------------------------------------------------------------------------

/// MQTT control packet type nibbles this profile understands on the inbound
/// path. Server-to-client CONNECT/SUBSCRIBE/PUBLISH-ack types are rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MqttPacketType {
    ConnAck = 2,
    Publish = 3,
    SubAck = 9,
    PingResp = 13,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MqttDecodeError {
    /// A reserved/unsupported control packet type arrived from the broker.
    UnsupportedPacket = 1,
    /// Reserved flag bits in the fixed header were set incorrectly.
    InvalidFlags = 2,
    /// The remaining-length varint was malformed or over four bytes.
    MalformedLength = 3,
    /// The declared packet exceeded `MAX_MQTT_PACKET_BYTES`.
    PacketTooLarge = 4,
    /// A PUBLISH used QoS 1 or 2 (unsupported by this subscribe profile).
    UnsupportedQos = 5,
    /// The PUBLISH topic was malformed, wildcarded, or oversized.
    InvalidTopic = 6,
    /// The PUBLISH payload exceeded `MAX_MQTT_PAYLOAD_BYTES`.
    PayloadTooLarge = 7,
    /// A packet body did not match its declared remaining length.
    Truncated = 8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DecodeStage {
    Type,
    Length,
    Body,
}

/// A decoded control packet event surfaced to the transport engine.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MqttInbound {
    ConnAck {
        session_present: bool,
        code: u8,
    },
    SubAck {
        packet_id: u16,
        return_code: u8,
    },
    /// A PUBLISH was validated and pushed into the message queue.
    Published {
        retained: bool,
    },
    PingResp,
}

/// Progress from one `push` call: bytes consumed and, if a full packet
/// completed within them, the decoded event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqttDecodeProgress {
    pub consumed: usize,
    pub event: Option<MqttInbound>,
}

/// Streaming, fixed-capacity decoder for inbound MQTT 3.1.1 control packets.
/// Bounds the remaining length before buffering any body bytes, so a malicious
/// broker cannot grow memory with the packet size it advertises.
pub struct MqttPacketDecoder {
    stage: DecodeStage,
    packet_type: u8,
    flags: u8,
    length: u32,
    length_shift: u32,
    length_bytes: u8,
    body_filled: usize,
    body: [u8; MAX_MQTT_PACKET_BYTES],
}

impl Default for MqttPacketDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl MqttPacketDecoder {
    pub const fn new() -> Self {
        Self {
            stage: DecodeStage::Type,
            packet_type: 0,
            flags: 0,
            length: 0,
            length_shift: 0,
            length_bytes: 0,
            body_filled: 0,
            body: [0; MAX_MQTT_PACKET_BYTES],
        }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Feed a chunk of received bytes. Consumes bytes up to the end of at most
    /// one completed packet; complete PUBLISH packets are pushed into `queue`.
    /// Call repeatedly until `consumed == 0` to drain a buffer.
    pub fn push(
        &mut self,
        input: &[u8],
        queue: &mut MqttMessageQueue,
    ) -> Result<MqttDecodeProgress, MqttDecodeError> {
        let mut consumed = 0;
        while consumed < input.len() {
            let byte = input[consumed];
            match self.stage {
                DecodeStage::Type => {
                    self.packet_type = byte >> 4;
                    self.flags = byte & 0x0f;
                    self.length = 0;
                    self.length_shift = 0;
                    self.length_bytes = 0;
                    self.body_filled = 0;
                    self.validate_header()?;
                    self.stage = DecodeStage::Length;
                    consumed += 1;
                }
                DecodeStage::Length => {
                    self.length |= u32::from(byte & 0x7f) << self.length_shift;
                    self.length_shift += 7;
                    self.length_bytes += 1;
                    consumed += 1;
                    if byte & 0x80 == 0 {
                        if self.length as usize > MAX_MQTT_PACKET_BYTES {
                            return Err(MqttDecodeError::PacketTooLarge);
                        }
                        if self.length == 0 {
                            let event = self.complete(queue)?;
                            return Ok(MqttDecodeProgress {
                                consumed,
                                event: Some(event),
                            });
                        }
                        self.stage = DecodeStage::Body;
                    } else if self.length_bytes == 4 {
                        return Err(MqttDecodeError::MalformedLength);
                    }
                }
                DecodeStage::Body => {
                    let need = self.length as usize - self.body_filled;
                    let avail = input.len() - consumed;
                    let take = need.min(avail);
                    self.body[self.body_filled..self.body_filled + take]
                        .copy_from_slice(&input[consumed..consumed + take]);
                    self.body_filled += take;
                    consumed += take;
                    if self.body_filled == self.length as usize {
                        let event = self.complete(queue)?;
                        return Ok(MqttDecodeProgress {
                            consumed,
                            event: Some(event),
                        });
                    }
                }
            }
        }
        Ok(MqttDecodeProgress {
            consumed,
            event: None,
        })
    }

    fn validate_header(&self) -> Result<(), MqttDecodeError> {
        match self.packet_type {
            x if x == MqttPacketType::ConnAck as u8 => self.expect_flags(0),
            x if x == MqttPacketType::SubAck as u8 => self.expect_flags(0),
            x if x == MqttPacketType::PingResp as u8 => self.expect_flags(0),
            x if x == MqttPacketType::Publish as u8 => {
                // QoS lives in bits 1-2; this profile accepts QoS 0 only.
                if (self.flags & 0x06) != 0 {
                    return Err(MqttDecodeError::UnsupportedQos);
                }
                Ok(())
            }
            _ => Err(MqttDecodeError::UnsupportedPacket),
        }
    }

    fn expect_flags(&self, expected: u8) -> Result<(), MqttDecodeError> {
        if self.flags == expected {
            Ok(())
        } else {
            Err(MqttDecodeError::InvalidFlags)
        }
    }

    fn complete(&mut self, queue: &mut MqttMessageQueue) -> Result<MqttInbound, MqttDecodeError> {
        let body = &self.body[..self.length as usize];
        let event = match self.packet_type {
            x if x == MqttPacketType::ConnAck as u8 => {
                if body.len() != 2 {
                    return Err(MqttDecodeError::Truncated);
                }
                if body[0] & 0xfe != 0 {
                    return Err(MqttDecodeError::InvalidFlags);
                }
                MqttInbound::ConnAck {
                    session_present: body[0] & 0x01 != 0,
                    code: body[1],
                }
            }
            x if x == MqttPacketType::SubAck as u8 => {
                if body.len() != 3 {
                    return Err(MqttDecodeError::Truncated);
                }
                MqttInbound::SubAck {
                    packet_id: u16::from(body[0]) << 8 | u16::from(body[1]),
                    return_code: body[2],
                }
            }
            x if x == MqttPacketType::PingResp as u8 => MqttInbound::PingResp,
            x if x == MqttPacketType::Publish as u8 => self.complete_publish(queue)?,
            _ => return Err(MqttDecodeError::UnsupportedPacket),
        };
        self.stage = DecodeStage::Type;
        Ok(event)
    }

    fn complete_publish(
        &self,
        queue: &mut MqttMessageQueue,
    ) -> Result<MqttInbound, MqttDecodeError> {
        let body = &self.body[..self.length as usize];
        if body.len() < 2 {
            return Err(MqttDecodeError::Truncated);
        }
        let topic_len = (usize::from(body[0]) << 8) | usize::from(body[1]);
        let topic_end = 2 + topic_len;
        if topic_end > body.len() {
            return Err(MqttDecodeError::Truncated);
        }
        let topic = &body[2..topic_end];
        // QoS 0 has no packet identifier; the rest is payload.
        let payload = &body[topic_end..];
        if topic_len > MAX_MQTT_TOPIC_BYTES {
            return Err(MqttDecodeError::InvalidTopic);
        }
        if payload.len() > MAX_MQTT_PAYLOAD_BYTES {
            return Err(MqttDecodeError::PayloadTooLarge);
        }
        // Reject wildcards, control characters, and invalid UTF-8 in the name.
        TopicFilter::from_bytes(topic).map_err(|_| MqttDecodeError::InvalidTopic)?;
        let retained = self.flags & 0x01 != 0;
        queue.push(topic, payload, retained);
        Ok(MqttInbound::Published { retained })
    }
}

// ---------------------------------------------------------------------------
// Service boundary
// ---------------------------------------------------------------------------

/// An OS-issued, generation-tagged MQTT session handle. Untrusted when supplied
/// by the VM; `AppMqttService` re-checks generation, live slot, and owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MqttSessionId(u32);

impl MqttSessionId {
    pub const fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u32 {
        self.0
    }

    const fn new(generation: u16, sequence: u16) -> Self {
        Self((generation as u32) << 16 | sequence as u32)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MqttError {
    Denied = 1,
    Unavailable = 2,
    Busy = 3,
    MalformedBroker = 4,
    Timeout = 5,
    Cancelled = 6,
    Connect = 7,
    Tls = 8,
    Protocol = 9,
    NotConnected = 10,
    TopicNotAllowed = 11,
    MessageTooLarge = 12,
    BufferTooLarge = 13,
    Disconnected = 14,
    StaleSession = 15,
    ForeignSession = 16,
}

/// App-visible session lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MqttPoll {
    /// TCP/TLS connect and CONNACK handshake in progress.
    Connecting,
    /// CONNACK accepted; ready to subscribe. No message queued.
    Connected,
    /// At least one message is queued for `read_message`.
    Message,
    /// The broker or network dropped the session cleanly.
    Disconnected,
    /// A terminal error ended the session.
    Failed(MqttError),
}

impl MqttPoll {
    /// Stable ABI state code for `mqtt_poll` (Host ABI minor 23, KOTO-0249).
    pub const fn state_code(self) -> i32 {
        match self {
            MqttPoll::Connecting => app_mqtt::STATE_CONNECTING,
            MqttPoll::Connected => app_mqtt::STATE_CONNECTED,
            MqttPoll::Message => app_mqtt::STATE_MESSAGE,
            MqttPoll::Disconnected => app_mqtt::STATE_DISCONNECTED,
            MqttPoll::Failed(_) => app_mqtt::STATE_FAILED,
        }
    }
}

/// Backend-reported session progress. The backend owns the socket, TLS, and
/// protocol engine and drains complete messages into the OS queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendMqttPoll {
    Connecting,
    Connected,
    Disconnected,
    Failed(MqttError),
}

/// OS-private transport seam. Implementations retain every network-stack object
/// and copy at most `dst.len()` bytes into caller-owned storage. `poll` also
/// pushes any newly received messages into the supplied bounded queue.
pub trait MqttBackend {
    fn available(&self) -> bool;
    fn connect(&mut self, session: MqttSessionId, origin: &MqttOrigin) -> Result<(), MqttError>;
    fn subscribe(&mut self, session: MqttSessionId, filter: &TopicFilter) -> Result<(), MqttError>;
    fn poll(&mut self, session: MqttSessionId, queue: &mut MqttMessageQueue) -> BackendMqttPoll;
    fn disconnect(&mut self, session: MqttSessionId);
}

/// Zero-sized backend for device/offline profiles that deliberately link no
/// socket, TLS, timer, or protocol implementation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnavailableMqttBackend;

impl MqttBackend for UnavailableMqttBackend {
    fn available(&self) -> bool {
        false
    }

    fn connect(&mut self, _: MqttSessionId, _: &MqttOrigin) -> Result<(), MqttError> {
        Err(MqttError::Unavailable)
    }

    fn subscribe(&mut self, _: MqttSessionId, _: &TopicFilter) -> Result<(), MqttError> {
        Err(MqttError::Unavailable)
    }

    fn poll(&mut self, _: MqttSessionId, _: &mut MqttMessageQueue) -> BackendMqttPoll {
        BackendMqttPoll::Failed(MqttError::Unavailable)
    }

    fn disconnect(&mut self, _: MqttSessionId) {}
}

#[derive(Debug)]
struct SessionSlot {
    id: MqttSessionId,
    owner: AppContext,
    broker_index: u8,
    started_ms: u64,
    connected: bool,
    state: MqttPoll,
    queue: MqttMessageQueue,
}

/// Fixed-capacity MQTT subscribe service. Enforces the manifest broker/topic
/// allowlist, one active session per application, a measured global session
/// cap, generation-tagged handles, the connect deadline, and bounded delivery.
#[derive(Debug)]
pub struct AppMqttService<B: MqttBackend> {
    backend: B,
    generation: u16,
    sequence: u16,
    slots: [Option<SessionSlot>; MAX_GLOBAL_MQTT_SESSIONS],
}

impl<B: MqttBackend> AppMqttService<B> {
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            generation: 1,
            sequence: 0,
            slots: [const { None }; MAX_GLOBAL_MQTT_SESSIONS],
        }
    }

    /// Open a session to a manifest-declared broker. At most one session per
    /// application and one globally.
    pub fn connect(
        &mut self,
        app: AppContext,
        brokers: &BrokerAllowlist,
        broker_index: usize,
        now_ms: u64,
    ) -> Result<MqttSessionId, MqttError> {
        if !self.backend.available() {
            return Err(MqttError::Unavailable);
        }
        if self.slots.iter().flatten().any(|slot| slot.owner == app) {
            return Err(MqttError::Busy);
        }
        let origin = brokers.get(broker_index).ok_or(MqttError::Denied)?;
        let slot_index = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(MqttError::Busy)?;
        self.sequence = self.sequence.wrapping_add(1);
        if self.sequence == 0 {
            self.sequence = 1;
        }
        let id = MqttSessionId::new(self.generation, self.sequence);
        self.backend.connect(id, &origin)?;
        self.slots[slot_index] = Some(SessionSlot {
            id,
            owner: app,
            broker_index: broker_index as u8,
            started_ms: now_ms,
            connected: false,
            state: MqttPoll::Connecting,
            queue: MqttMessageQueue::new(),
        });
        Ok(id)
    }

    /// Subscribe to one manifest-declared exact topic filter. Only valid once
    /// the CONNACK handshake has completed.
    pub fn subscribe(
        &mut self,
        app: AppContext,
        id: MqttSessionId,
        topics: &TopicFilterSet,
        topic_index: usize,
        now_ms: u64,
    ) -> Result<(), MqttError> {
        let index = self.slot_index(app, id)?;
        self.advance(index, now_ms);
        let slot = self.slots[index].as_ref().expect("located slot");
        match slot.state {
            MqttPoll::Failed(error) => return Err(error),
            MqttPoll::Disconnected => return Err(MqttError::Disconnected),
            MqttPoll::Connecting => return Err(MqttError::NotConnected),
            MqttPoll::Connected | MqttPoll::Message => {}
        }
        let filter = topics.get(topic_index).ok_or(MqttError::TopicNotAllowed)?;
        self.backend.subscribe(id, &filter)
    }

    /// Advance the session and return its current state.
    pub fn poll(
        &mut self,
        app: AppContext,
        id: MqttSessionId,
        now_ms: u64,
    ) -> Result<MqttPoll, MqttError> {
        let index = self.slot_index(app, id)?;
        self.advance(index, now_ms);
        Ok(self.slots[index].as_ref().expect("located slot").state)
    }

    /// Copy the oldest queued message into caller-owned buffers. Returns
    /// `Ok(None)` when no message is queued. A message larger than either buffer
    /// is consumed and reported as `MessageTooLarge`, never as a partial read.
    pub fn read_message(
        &mut self,
        app: AppContext,
        id: MqttSessionId,
        topic: &mut [u8],
        payload: &mut [u8],
    ) -> Result<Option<MqttMessage>, MqttError> {
        if topic.len() > MAX_MQTT_TOPIC_BYTES || payload.len() > MAX_MQTT_PAYLOAD_BYTES {
            return Err(MqttError::BufferTooLarge);
        }
        let index = self.slot_index(app, id)?;
        let slot = self.slots[index].as_mut().expect("located slot");
        let message = slot.queue.pop(topic, payload)?;
        if slot.queue.is_empty() && matches!(slot.state, MqttPoll::Message) {
            slot.state = MqttPoll::Connected;
        }
        Ok(message)
    }

    /// Full lengths `(topic_len, payload_len)` of the oldest queued message
    /// without consuming it, so the app can size its buffers before
    /// `read_message`. `Ok(None)` when the queue is empty. Idempotent
    /// (Host ABI minor 23, KOTO-0249).
    pub fn peek(
        &self,
        app: AppContext,
        id: MqttSessionId,
    ) -> Result<Option<(usize, usize)>, MqttError> {
        let index = self.slot_index(app, id)?;
        Ok(self.slots[index]
            .as_ref()
            .expect("located slot")
            .queue
            .front_lengths())
    }

    /// Number of messages dropped by the OS queue overflow policy for a session.
    pub fn dropped(&self, app: AppContext, id: MqttSessionId) -> Result<u32, MqttError> {
        let index = self.slot_index(app, id)?;
        Ok(self.slots[index]
            .as_ref()
            .expect("located slot")
            .queue
            .dropped())
    }

    /// Broker allowlist index a session is bound to (for redacted diagnostics).
    pub fn broker_index(&self, app: AppContext, id: MqttSessionId) -> Result<u8, MqttError> {
        let index = self.slot_index(app, id)?;
        Ok(self.slots[index]
            .as_ref()
            .expect("located slot")
            .broker_index)
    }

    /// Disconnect and release the session, zeroizing its queued messages.
    pub fn disconnect(&mut self, app: AppContext, id: MqttSessionId) -> Result<(), MqttError> {
        let index = self.slot_index(app, id)?;
        self.backend.disconnect(id);
        if let Some(mut slot) = self.slots[index].take() {
            slot.queue.clear();
        }
        Ok(())
    }

    /// Cancel and zeroize every session on app exit, capability loss, network
    /// generation change, permission revocation, or service teardown. Advancing
    /// the generation invalidates every previously issued session ID.
    pub fn teardown(&mut self) {
        for slot in self.slots.iter_mut() {
            if let Some(mut session) = slot.take() {
                self.backend.disconnect(session.id);
                session.queue.clear();
            }
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    fn advance(&mut self, index: usize, now_ms: u64) {
        let (id, timed_out) = {
            let slot = self.slots[index].as_ref().expect("located slot");
            if matches!(slot.state, MqttPoll::Failed(_) | MqttPoll::Disconnected) {
                return;
            }
            let timed_out = !slot.connected
                && now_ms.saturating_sub(slot.started_ms) > u64::from(MQTT_CONNECT_DEADLINE_MS);
            (slot.id, timed_out)
        };
        if timed_out {
            self.backend.disconnect(id);
            let slot = self.slots[index].as_mut().expect("located slot");
            slot.state = MqttPoll::Failed(MqttError::Timeout);
            slot.queue.clear();
            return;
        }
        // Split the borrow: poll the backend against the slot's own queue.
        let mut queue =
            core::mem::take(&mut self.slots[index].as_mut().expect("located slot").queue);
        let backend_state = self.backend.poll(id, &mut queue);
        let slot = self.slots[index].as_mut().expect("located slot");
        slot.queue = queue;
        match backend_state {
            BackendMqttPoll::Connecting => {}
            BackendMqttPoll::Connected => slot.connected = true,
            BackendMqttPoll::Disconnected => {
                slot.state = MqttPoll::Disconnected;
                slot.queue.clear();
                return;
            }
            BackendMqttPoll::Failed(error) => {
                slot.state = MqttPoll::Failed(error);
                slot.queue.clear();
                return;
            }
        }
        slot.state = if !slot.queue.is_empty() {
            MqttPoll::Message
        } else if slot.connected {
            MqttPoll::Connected
        } else {
            MqttPoll::Connecting
        };
    }

    fn slot_index(&self, app: AppContext, id: MqttSessionId) -> Result<usize, MqttError> {
        if (id.raw() >> 16) as u16 != self.generation {
            return Err(MqttError::StaleSession);
        }
        let index = self
            .slots
            .iter()
            .position(|slot| slot.as_ref().is_some_and(|slot| slot.id == id))
            .ok_or(MqttError::StaleSession)?;
        if self.slots[index].as_ref().expect("located slot").owner != app {
            return Err(MqttError::ForeignSession);
        }
        Ok(index)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_remaining_length(mut value: usize, out: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value > 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if value == 0 {
                break;
            }
        }
    }

    fn connack(session_present: bool, code: u8) -> Vec<u8> {
        vec![0x20, 0x02, session_present as u8, code]
    }

    fn suback(packet_id: u16, return_code: u8) -> Vec<u8> {
        vec![
            0x90,
            0x03,
            (packet_id >> 8) as u8,
            packet_id as u8,
            return_code,
        ]
    }

    fn publish(topic: &str, payload: &[u8], retained: bool) -> Vec<u8> {
        let mut body = Vec::new();
        body.push((topic.len() >> 8) as u8);
        body.push(topic.len() as u8);
        body.extend_from_slice(topic.as_bytes());
        body.extend_from_slice(payload);
        let mut packet = vec![0x30 | retained as u8];
        encode_remaining_length(body.len(), &mut packet);
        packet.extend_from_slice(&body);
        packet
    }

    fn pingresp() -> Vec<u8> {
        vec![0xd0, 0x00]
    }

    // --- broker origin -----------------------------------------------------

    #[test]
    fn broker_origin_canonical_and_rejections() {
        let origin = MqttOrigin::parse("mqtts://broker.example.com").unwrap();
        assert_eq!(origin.hostname(), "broker.example.com");
        assert_eq!(origin.port(), 8883);
        assert_eq!(origin.scheme(), MqttScheme::Mqtts);

        let dev = MqttOrigin::parse("mqtt://192.0.2.10:1884").unwrap();
        assert_eq!(dev.port(), 1884);
        assert_eq!(dev.scheme(), MqttScheme::Mqtt);
        assert!(!dev.scheme().release_allowed());

        assert_eq!(
            MqttOrigin::parse("mqtts://broker.example.com:8883"),
            Err(BrokerError::NonCanonical)
        );
        assert_eq!(
            MqttOrigin::parse("mqtts://*.example.com"),
            Err(BrokerError::Wildcard)
        );
        assert_eq!(
            MqttOrigin::parse("mqtts://user@example.com"),
            Err(BrokerError::UserInfo)
        );
        assert_eq!(
            MqttOrigin::parse("mqtts://broker.example.com/path"),
            Err(BrokerError::Malformed)
        );
        assert_eq!(
            MqttOrigin::parse("wss://broker.example.com"),
            Err(BrokerError::UnsupportedScheme)
        );
        assert_eq!(
            MqttOrigin::parse("mqtts://Broker.Example.com"),
            Err(BrokerError::InvalidHostname)
        );
        assert_eq!(
            MqttOrigin::parse("mqtts://broker.example.com:0"),
            Err(BrokerError::InvalidPort)
        );
    }

    // --- topic filters -----------------------------------------------------

    #[test]
    fn topic_filter_exact_only() {
        assert!(TopicFilter::parse("home/sensors/temp").is_ok());
        assert_eq!(TopicFilter::parse("home/+/temp"), Err(TopicError::Wildcard));
        assert_eq!(TopicFilter::parse("home/#"), Err(TopicError::Wildcard));
        assert_eq!(TopicFilter::parse(""), Err(TopicError::Empty));
        assert_eq!(
            TopicFilter::from_bytes(&[b'a', 0x00, b'b']),
            Err(TopicError::ControlCharacter)
        );
        assert_eq!(
            TopicFilter::from_bytes(&[0xff, 0xfe]),
            Err(TopicError::InvalidUtf8)
        );
        let long = "a".repeat(MAX_MQTT_TOPIC_BYTES + 1);
        assert_eq!(TopicFilter::parse(&long), Err(TopicError::TooLong));
    }

    #[test]
    fn topic_set_dedup_and_overflow() {
        let mut set = TopicFilterSet::empty();
        let a = TopicFilter::parse("a/b").unwrap();
        set.push(a).unwrap();
        assert_eq!(set.push(a), Err(TopicSetError::Duplicate));
        for index in 0..MAX_MQTT_TOPIC_FILTERS - 1 {
            let filter = TopicFilter::parse(&format!("t/{index}")).unwrap();
            set.push(filter).unwrap();
        }
        let overflow = TopicFilter::parse("t/overflow").unwrap();
        assert_eq!(set.push(overflow), Err(TopicSetError::Full));
        assert_eq!(set.position(b"a/b"), Some(0));
        assert!(set.contains_topic(b"a/b"));
        assert!(!set.contains_topic(b"a/c"));
    }

    #[test]
    fn broker_allowlist_dedup_and_overflow() {
        let mut list = BrokerAllowlist::empty();
        let one = MqttOrigin::parse("mqtts://one.example").unwrap();
        list.push(one).unwrap();
        assert_eq!(list.push(one), Err(BrokerListError::Duplicate));
        list.push(MqttOrigin::parse("mqtts://two.example").unwrap())
            .unwrap();
        assert_eq!(
            list.push(MqttOrigin::parse("mqtts://three.example").unwrap()),
            Err(BrokerListError::Full)
        );
        assert_eq!(list.position(&one), Some(0));
    }

    // --- manifest ----------------------------------------------------------

    const MANIFEST: &str = r#"{
        "name": "telemetry",
        "permissions": {
            "network": { "origins": ["https://api.example.com"] },
            "mqtt": {
                "brokers": ["mqtts://broker.example.com"],
                "topics": ["home/sensors/temp", "home/sensors/humidity"]
            }
        }
    }"#;

    #[test]
    fn manifest_declares_broker_and_topics() {
        let permission = parse_manifest_mqtt_permission(MANIFEST, 2).unwrap();
        assert!(permission.is_declared());
        assert_eq!(permission.brokers.len(), 1);
        assert_eq!(permission.topics.len(), 2);
        assert_eq!(
            permission.brokers.get(0).unwrap().hostname(),
            "broker.example.com"
        );
        assert!(permission.topics.contains_topic(b"home/sensors/temp"));
    }

    #[test]
    fn manifest_absent_is_default_denied() {
        let permission = parse_manifest_mqtt_permission(r#"{"name":"x"}"#, 2).unwrap();
        assert!(!permission.is_declared());
        assert!(permission.brokers.is_empty());
        assert!(permission.topics.is_empty());
    }

    #[test]
    fn manifest_rejections() {
        // Unsupported version.
        assert_eq!(
            parse_manifest_mqtt_permission(MANIFEST, 1),
            Err(ManifestMqttError::UnsupportedVersion)
        );
        // Wildcard topic.
        let wild = r#"{"permissions":{"mqtt":{"brokers":["mqtts://b.example"],"topics":["a/#"]}}}"#;
        assert_eq!(
            parse_manifest_mqtt_permission(wild, 2),
            Err(ManifestMqttError::InvalidTopic)
        );
        // Duplicate broker.
        let dup = r#"{"permissions":{"mqtt":{"brokers":["mqtts://b.example","mqtts://b.example"],"topics":["a/b"]}}}"#;
        assert_eq!(
            parse_manifest_mqtt_permission(dup, 2),
            Err(ManifestMqttError::InvalidBroker)
        );
        // Missing topics.
        let no_topics = r#"{"permissions":{"mqtt":{"brokers":["mqtts://b.example"]}}}"#;
        assert_eq!(
            parse_manifest_mqtt_permission(no_topics, 2),
            Err(ManifestMqttError::InvalidShape)
        );
        // A nested `mqtt` key inside an unknown member must not be mistaken for
        // the root declaration.
        let nested = r#"{"meta":{"mqtt":{"brokers":["mqtts://evil.example"],"topics":["a/b"]}}}"#;
        assert!(!parse_manifest_mqtt_permission(nested, 2)
            .unwrap()
            .is_declared());
        // Trailing garbage.
        assert_eq!(
            parse_manifest_mqtt_permission(r#"{"name":"x"} trailing"#, 2),
            Err(ManifestMqttError::InvalidJson)
        );
    }

    // --- message queue -----------------------------------------------------

    #[test]
    fn queue_drops_oldest_on_overflow() {
        let mut queue = MqttMessageQueue::new();
        for index in 0..MAX_MQTT_MESSAGE_QUEUE + 2 {
            queue.push(b"t", &[index as u8], false);
        }
        assert_eq!(queue.len(), MAX_MQTT_MESSAGE_QUEUE);
        assert_eq!(queue.dropped(), 2);
        // Oldest two (payloads 0,1) were evicted; the front is now payload 2.
        let mut topic = [0u8; MAX_MQTT_TOPIC_BYTES];
        let mut payload = [0u8; MAX_MQTT_PAYLOAD_BYTES];
        let message = queue.pop(&mut topic, &mut payload).unwrap().unwrap();
        assert_eq!(payload[..message.payload_len as usize], [2]);
    }

    #[test]
    fn queue_reports_oversize_without_partial() {
        let mut queue = MqttMessageQueue::new();
        queue.push(b"topic", b"payload", false);
        let mut topic = [0u8; 3];
        let mut payload = [0u8; MAX_MQTT_PAYLOAD_BYTES];
        assert_eq!(
            queue.pop(&mut topic, &mut payload),
            Err(MqttError::MessageTooLarge)
        );
        // The offending message was consumed, not left as a partial.
        assert!(queue.is_empty());
    }

    // --- packet decoder ----------------------------------------------------

    fn decode_all(bytes: &[u8], chunk: usize) -> (Vec<MqttInbound>, MqttMessageQueue) {
        let mut decoder = MqttPacketDecoder::new();
        let mut queue = MqttMessageQueue::new();
        let mut events = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let end = (at + chunk).min(bytes.len());
            let progress = decoder.push(&bytes[at..end], &mut queue).unwrap();
            if let Some(event) = progress.event {
                events.push(event);
            }
            if progress.consumed == 0 {
                at = end;
            } else {
                at += progress.consumed;
            }
        }
        (events, queue)
    }

    #[test]
    fn decoder_handles_stream_at_every_chunk_size() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&connack(false, 0));
        stream.extend_from_slice(&suback(1, 0));
        stream.extend_from_slice(&publish("home/sensors/temp", b"21.5", true));
        stream.extend_from_slice(&pingresp());

        for chunk in 1..=stream.len() {
            let (events, queue) = decode_all(&stream, chunk);
            assert_eq!(
                events,
                vec![
                    MqttInbound::ConnAck {
                        session_present: false,
                        code: 0
                    },
                    MqttInbound::SubAck {
                        packet_id: 1,
                        return_code: 0
                    },
                    MqttInbound::Published { retained: true },
                    MqttInbound::PingResp,
                ],
                "chunk size {chunk}"
            );
            assert_eq!(queue.len(), 1, "chunk size {chunk}");
        }
    }

    #[test]
    fn decoder_rejects_unsupported_and_oversized() {
        let mut queue = MqttMessageQueue::new();

        // QoS 1 PUBLISH.
        let mut decoder = MqttPacketDecoder::new();
        assert_eq!(
            decoder.push(&[0x32, 0x00], &mut queue),
            Err(MqttDecodeError::UnsupportedQos)
        );

        // Reserved CONNECT packet from server.
        let mut decoder = MqttPacketDecoder::new();
        assert_eq!(
            decoder.push(&[0x10, 0x00], &mut queue),
            Err(MqttDecodeError::UnsupportedPacket)
        );

        // Oversized remaining length (300 > 256 cap) is rejected before body.
        let mut decoder = MqttPacketDecoder::new();
        let mut bytes = vec![0x30];
        encode_remaining_length(300, &mut bytes);
        assert_eq!(
            decoder.push(&bytes, &mut queue),
            Err(MqttDecodeError::PacketTooLarge)
        );

        // A PUBLISH whose payload exceeds the profile maximum.
        let mut decoder = MqttPacketDecoder::new();
        let big = publish("t", &[0u8; MAX_MQTT_PAYLOAD_BYTES + 1], false);
        assert_eq!(
            decoder.push(&big, &mut queue),
            Err(MqttDecodeError::PayloadTooLarge)
        );

        // A wildcard topic name in a PUBLISH is rejected.
        let mut decoder = MqttPacketDecoder::new();
        let wild = publish("home/+/x", b"1", false);
        assert_eq!(
            decoder.push(&wild, &mut queue),
            Err(MqttDecodeError::InvalidTopic)
        );
    }

    // --- service -----------------------------------------------------------

    struct FakeBackend {
        available: bool,
        script: Vec<u8>,
        cursor: usize,
        chunk: usize,
        decoder: MqttPacketDecoder,
        connected: bool,
        subscriptions: u8,
        disconnects: u8,
        fail_poll: Option<MqttError>,
    }

    impl FakeBackend {
        fn new(script: Vec<u8>) -> Self {
            Self {
                available: true,
                script,
                cursor: 0,
                chunk: 4,
                decoder: MqttPacketDecoder::new(),
                connected: false,
                subscriptions: 0,
                disconnects: 0,
                fail_poll: None,
            }
        }
    }

    impl MqttBackend for FakeBackend {
        fn available(&self) -> bool {
            self.available
        }

        fn connect(&mut self, _: MqttSessionId, _: &MqttOrigin) -> Result<(), MqttError> {
            Ok(())
        }

        fn subscribe(&mut self, _: MqttSessionId, _: &TopicFilter) -> Result<(), MqttError> {
            self.subscriptions += 1;
            Ok(())
        }

        fn poll(&mut self, _: MqttSessionId, queue: &mut MqttMessageQueue) -> BackendMqttPoll {
            if let Some(error) = self.fail_poll {
                return BackendMqttPoll::Failed(error);
            }
            let end = (self.cursor + self.chunk).min(self.script.len());
            while self.cursor < end {
                let progress = match self.decoder.push(&self.script[self.cursor..end], queue) {
                    Ok(progress) => progress,
                    Err(_) => return BackendMqttPoll::Failed(MqttError::Protocol),
                };
                if progress.consumed == 0 {
                    break;
                }
                self.cursor += progress.consumed;
                if let Some(MqttInbound::ConnAck { code, .. }) = progress.event {
                    if code == 0 {
                        self.connected = true;
                    } else {
                        return BackendMqttPoll::Failed(MqttError::Connect);
                    }
                }
            }
            if self.connected {
                BackendMqttPoll::Connected
            } else {
                BackendMqttPoll::Connecting
            }
        }

        fn disconnect(&mut self, _: MqttSessionId) {
            self.disconnects += 1;
        }
    }

    fn app() -> AppContext {
        AppContext {
            app_id: 7,
            generation: 1,
        }
    }

    fn brokers() -> BrokerAllowlist {
        let mut list = BrokerAllowlist::empty();
        list.push(MqttOrigin::parse("mqtts://broker.example.com").unwrap())
            .unwrap();
        list
    }

    fn topics() -> TopicFilterSet {
        let mut set = TopicFilterSet::empty();
        set.push(TopicFilter::parse("home/sensors/temp").unwrap())
            .unwrap();
        set
    }

    #[test]
    fn service_connect_subscribe_receive() {
        let mut script = Vec::new();
        script.extend_from_slice(&connack(false, 0));
        script.extend_from_slice(&suback(1, 0));
        script.extend_from_slice(&publish("home/sensors/temp", b"21.5", false));

        let mut service = AppMqttService::new(FakeBackend::new(script));
        let brokers = brokers();
        let topics = topics();
        let id = service.connect(app(), &brokers, 0, 0).unwrap();
        assert_eq!(service.broker_index(app(), id).unwrap(), 0);

        // Drive the backend until CONNACK lands.
        let mut now = 0;
        loop {
            now += 10;
            match service.poll(app(), id, now).unwrap() {
                MqttPoll::Connecting => continue,
                MqttPoll::Connected | MqttPoll::Message => break,
                other => panic!("unexpected {other:?}"),
            }
        }

        service.subscribe(app(), id, &topics, 0, now).unwrap();

        // Drive until a message is queued.
        loop {
            now += 10;
            if matches!(service.poll(app(), id, now).unwrap(), MqttPoll::Message) {
                break;
            }
        }
        let mut topic = [0u8; MAX_MQTT_TOPIC_BYTES];
        let mut payload = [0u8; MAX_MQTT_PAYLOAD_BYTES];
        let message = service
            .read_message(app(), id, &mut topic, &mut payload)
            .unwrap()
            .unwrap();
        assert_eq!(&topic[..message.topic_len as usize], b"home/sensors/temp");
        assert_eq!(&payload[..message.payload_len as usize], b"21.5");
        // Queue drained -> back to Connected.
        assert_eq!(service.poll(app(), id, now).unwrap(), MqttPoll::Connected);
    }

    #[test]
    fn service_enforces_ownership_and_bounds() {
        let mut service = AppMqttService::new(FakeBackend::new(connack(false, 0)));
        let brokers = brokers();
        let topics = topics();

        // Out-of-range broker is denied.
        assert_eq!(
            service.connect(app(), &brokers, 3, 0),
            Err(MqttError::Denied)
        );

        let id = service.connect(app(), &brokers, 0, 0).unwrap();

        // One session per app.
        assert_eq!(service.connect(app(), &brokers, 0, 0), Err(MqttError::Busy));

        // A foreign app cannot touch the session.
        let intruder = AppContext {
            app_id: 99,
            generation: 1,
        };
        assert_eq!(
            service.poll(intruder, id, 0),
            Err(MqttError::ForeignSession)
        );

        // Oversized caller buffer is rejected.
        let mut topic = [0u8; MAX_MQTT_TOPIC_BYTES + 1];
        let mut payload = [0u8; MAX_MQTT_PAYLOAD_BYTES];
        assert_eq!(
            service.read_message(app(), id, &mut topic, &mut payload),
            Err(MqttError::BufferTooLarge)
        );

        // Subscribing before CONNACK (no bytes from the broker yet) is rejected.
        let mut pending = AppMqttService::new(FakeBackend::new(Vec::new()));
        let pending_id = pending.connect(app(), &brokers, 0, 0).unwrap();
        assert_eq!(
            pending.subscribe(app(), pending_id, &topics, 0, 0),
            Err(MqttError::NotConnected)
        );
    }

    #[test]
    fn service_connect_deadline_times_out() {
        // Empty script: the backend never sends CONNACK.
        let mut service = AppMqttService::new(FakeBackend::new(Vec::new()));
        let brokers = brokers();
        let id = service.connect(app(), &brokers, 0, 0).unwrap();
        assert_eq!(
            service.poll(app(), id, 1_000).unwrap(),
            MqttPoll::Connecting
        );
        assert_eq!(
            service.poll(app(), id, u64::from(MQTT_CONNECT_DEADLINE_MS) + 1),
            Ok(MqttPoll::Failed(MqttError::Timeout))
        );
    }

    #[test]
    fn teardown_invalidates_sessions() {
        let mut service = AppMqttService::new(FakeBackend::new(connack(false, 0)));
        let brokers = brokers();
        let id = service.connect(app(), &brokers, 0, 0).unwrap();
        service.teardown();
        assert_eq!(service.poll(app(), id, 0), Err(MqttError::StaleSession));
        // A new session issues under the new generation.
        let again = service.connect(app(), &brokers, 0, 0).unwrap();
        assert_ne!(again.raw() >> 16, id.raw() >> 16);
    }

    #[test]
    fn unavailable_backend_returns_unavailable() {
        let mut service = AppMqttService::new(UnavailableMqttBackend);
        let brokers = brokers();
        assert_eq!(
            service.connect(app(), &brokers, 0, 0),
            Err(MqttError::Unavailable)
        );
    }

    #[test]
    fn service_reports_backend_failure() {
        let mut backend = FakeBackend::new(Vec::new());
        backend.fail_poll = Some(MqttError::Disconnected);
        let mut service = AppMqttService::new(backend);
        let brokers = brokers();
        let id = service.connect(app(), &brokers, 0, 0).unwrap();
        assert_eq!(
            service.poll(app(), id, 10),
            Ok(MqttPoll::Failed(MqttError::Disconnected))
        );
    }
}
