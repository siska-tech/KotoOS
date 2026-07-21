//! Bounded application Fetch service (KOTO-0245).
//!
//! This module is deliberately transport-neutral and `no_std`. Applications
//! receive request IDs and copied bytes only; the OS-owned backend retains DNS,
//! socket, and TLS state. A simulator fake and the firmware network task can
//! therefore share the same permission and lifecycle policy.

pub const MAX_FETCH_ORIGINS: usize = 4;
pub const MAX_FETCH_HOSTNAME_BYTES: usize = 253;
pub const MAX_FETCH_URL_BYTES: usize = 384;
pub const MAX_FETCH_READ_BYTES: usize = 512;
/// Device transport producer chunk. VM reads may request up to
/// `MAX_FETCH_READ_BYTES`; a backend is free to satisfy them incrementally.
pub const FETCH_TRANSPORT_CHUNK_BYTES: usize = 128;
pub const MAX_FETCH_HEADER_BYTES: usize = 1_024;
pub const MAX_FETCH_TOTAL_BYTES: u32 = 65_536;
pub const MAX_FETCH_DURATION_MS: u32 = 15_000;
pub const MAX_GLOBAL_FETCH_REQUESTS: usize = 2;

/// Release-profile destination filter, applied by a backend after every DNS
/// answer (and again before connect). Development local-network overrides live
/// outside this portable release predicate.
pub fn release_ipv4_allowed(address: [u8; 4]) -> bool {
    let [a, b, _, _] = address;
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (b & 0xc0) == 0x40)
        || (a == 169 && b == 254)
        || (a == 172 && (b & 0xf0) == 16)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 224)
}

pub fn release_ipv6_allowed(address: [u8; 16]) -> bool {
    let unspecified = address == [0; 16];
    let loopback = address[..15].iter().all(|byte| *byte == 0) && address[15] == 1;
    let multicast = address[0] == 0xff;
    let link_local = address[0] == 0xfe && (address[1] & 0xc0) == 0x80;
    let unique_local = (address[0] & 0xfe) == 0xfc;
    !(unspecified || loopback || multicast || link_local || unique_local)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FetchScheme {
    Http = 0,
    Https = 1,
}

impl FetchScheme {
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OriginError {
    Malformed,
    UnsupportedScheme,
    HostnameTooLong,
    InvalidHostname,
    InvalidPort,
    UserInfo,
    Wildcard,
    NonCanonical,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct FetchOrigin {
    scheme: FetchScheme,
    hostname: [u8; MAX_FETCH_HOSTNAME_BYTES],
    hostname_len: u8,
    port: u16,
}

/// Borrowed, allocation-free URL fields consumed by the OS-private transport.
/// An empty suffix denotes `/`; a suffix beginning with `?` is sent as `/?...`.
/// Keeping the original suffix borrowed prevents a second URL grammar from
/// appearing in the network executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchUrlTarget<'a> {
    origin: FetchOrigin,
    request_target_suffix: &'a str,
}

impl<'a> FetchUrlTarget<'a> {
    pub const fn origin(&self) -> FetchOrigin {
        self.origin
    }

    pub fn hostname(&self) -> &str {
        self.origin.hostname()
    }

    pub const fn port(&self) -> u16 {
        self.origin.port()
    }

    pub const fn scheme(&self) -> FetchScheme {
        self.origin.scheme()
    }

    pub const fn request_target_suffix(&self) -> &'a str {
        self.request_target_suffix
    }
}

/// Parses the complete Fetch URL once for both policy and transport use.
/// V1 requires wire-safe ASCII and rejects fragments, whose client-side
/// semantics must never be confused with the HTTP request target.
pub fn parse_fetch_url(url: &str) -> Result<FetchUrlTarget<'_>, OriginError> {
    if url.is_empty()
        || url.len() > MAX_FETCH_URL_BYTES
        || url.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
        || url.contains('#')
    {
        return Err(OriginError::Malformed);
    }
    let scheme_end = url.find("://").ok_or(OriginError::Malformed)?;
    let authority_start = scheme_end + 3;
    let authority_end = url[authority_start..]
        .find(['/', '?'])
        .map_or(url.len(), |end| authority_start + end);
    if authority_end == authority_start {
        return Err(OriginError::Malformed);
    }
    let origin = FetchOrigin::parse(&url[..authority_end])?;
    Ok(FetchUrlTarget {
        origin,
        request_target_suffix: &url[authority_end..],
    })
}

const HTTP_GET_PREFIX: &[u8] = b"GET ";
const HTTP_VERSION_AND_HOST: &[u8] = b" HTTP/1.1\r\nHost: ";
const HTTP_FIXED_HEADERS: &[u8] = b"\r\nAccept: application/json\r\nConnection: close\r\n\r\n";

const HTTP_HEADER_NAME_SEP: &[u8] = b": ";
const HTTP_CRLF: &[u8] = b"\r\n";

/// One OS-injected request header written as `<name>: <prefix><secret>`. The
/// split prefix/secret lets `Authorization: Bearer <token>` be emitted directly
/// into the transport buffer without pre-concatenating the secret elsewhere.
struct CredentialHeader<'a> {
    name: &'a [u8],
    value_prefix: &'a [u8],
    value_secret: &'a [u8],
}

impl CredentialHeader<'_> {
    /// Wire length of `\r\n<name>: <prefix><secret>`.
    fn encoded_len(&self) -> usize {
        HTTP_CRLF.len()
            + self.name.len()
            + HTTP_HEADER_NAME_SEP.len()
            + self.value_prefix.len()
            + self.value_secret.len()
    }

    /// Rejects any name/value byte that could break out of the header line.
    /// The vault already validates its stored bytes; this is defense in depth
    /// so no header injection is possible even if a caller supplies raw bytes.
    fn is_wire_safe(&self) -> bool {
        let name_ok = !self.name.is_empty() && self.name.iter().all(|b| is_http_token_byte(*b));
        let value_ok = self
            .value_prefix
            .iter()
            .chain(self.value_secret.iter())
            .all(|b| (0x20..=0x7e).contains(b));
        name_ok && value_ok
    }
}

/// RFC 7230 token characters (a header field name).
fn is_http_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

/// Encodes the complete v1 HTTP GET head into caller-owned transport storage.
/// The function validates and sizes the request before writing, so an
/// undersized destination is left untouched. The empty URL suffix is `/` and
/// a query-only suffix is prefixed with `/` on the wire.
pub fn encode_fetch_get_request(url: &str, dst: &mut [u8]) -> Result<usize, FetchError> {
    encode_fetch_get_request_inner(url, None, dst)
}

/// Encodes the v1 HTTP GET head with one OS-owned credential injected from the
/// vault (KOTO-0248). Only the two Fetch credential shapes are valid here; an
/// MQTT login is refused. The credential header is written inside the same
/// transport buffer the caller zeroizes after the exchange, so the secret never
/// needs a second staging copy. This is the OS-private transport injection
/// point: policy (origin, app context, TLS) is enforced by the vault and the
/// controller before this is called.
pub fn encode_fetch_get_request_with_injection(
    url: &str,
    injection: &crate::vault::CredentialInjection<'_>,
    dst: &mut [u8],
) -> Result<usize, FetchError> {
    use crate::vault::CredentialInjection;
    let header = match injection {
        CredentialInjection::BearerToken { token } => CredentialHeader {
            name: b"Authorization",
            value_prefix: b"Bearer ",
            value_secret: token,
        },
        CredentialInjection::ApiKeyHeader { name, value } => CredentialHeader {
            name,
            value_prefix: b"",
            value_secret: value,
        },
        // A Fetch request can never carry an MQTT login.
        CredentialInjection::MqttLogin { .. } => return Err(FetchError::Denied),
    };
    if !header.is_wire_safe() {
        return Err(FetchError::Denied);
    }
    encode_fetch_get_request_inner(url, Some(header), dst)
}

fn encode_fetch_get_request_inner(
    url: &str,
    header: Option<CredentialHeader<'_>>,
    dst: &mut [u8],
) -> Result<usize, FetchError> {
    let target = parse_fetch_url(url).map_err(|_| FetchError::MalformedUrl)?;
    let suffix = target.request_target_suffix().as_bytes();
    let target_prefix = usize::from(suffix.is_empty() || suffix.starts_with(b"?"));
    let explicit_port = target.port() != target.scheme().default_port();
    let port_digits = if explicit_port {
        decimal_u16_digits(target.port())
    } else {
        0
    };
    let header_len = header.as_ref().map_or(0, CredentialHeader::encoded_len);
    let required = HTTP_GET_PREFIX.len()
        + target_prefix
        + suffix.len()
        + HTTP_VERSION_AND_HOST.len()
        + target.hostname().len()
        + usize::from(explicit_port)
        + port_digits
        + header_len
        + HTTP_FIXED_HEADERS.len();
    if required > dst.len() {
        return Err(FetchError::BufferTooLarge);
    }

    let mut at = 0;
    append_bytes(dst, &mut at, HTTP_GET_PREFIX);
    if target_prefix != 0 {
        append_bytes(dst, &mut at, b"/");
    }
    append_bytes(dst, &mut at, suffix);
    append_bytes(dst, &mut at, HTTP_VERSION_AND_HOST);
    append_bytes(dst, &mut at, target.hostname().as_bytes());
    if explicit_port {
        append_bytes(dst, &mut at, b":");
        append_decimal_u16(dst, &mut at, target.port(), port_digits);
    }
    if let Some(header) = header.as_ref() {
        // Insert the credential header line between the Host line and the fixed
        // Accept/Connection headers: `...Host: <host>` + `\r\n<name>: <value>`
        // + `\r\nAccept: ...\r\n\r\n`.
        append_bytes(dst, &mut at, HTTP_CRLF);
        append_bytes(dst, &mut at, header.name);
        append_bytes(dst, &mut at, HTTP_HEADER_NAME_SEP);
        append_bytes(dst, &mut at, header.value_prefix);
        append_bytes(dst, &mut at, header.value_secret);
    }
    append_bytes(dst, &mut at, HTTP_FIXED_HEADERS);
    debug_assert_eq!(at, required);
    Ok(required)
}

const fn decimal_u16_digits(value: u16) -> usize {
    if value >= 10_000 {
        5
    } else if value >= 1_000 {
        4
    } else if value >= 100 {
        3
    } else if value >= 10 {
        2
    } else {
        1
    }
}

fn append_bytes(dst: &mut [u8], at: &mut usize, src: &[u8]) {
    dst[*at..*at + src.len()].copy_from_slice(src);
    *at += src.len();
}

fn append_decimal_u16(dst: &mut [u8], at: &mut usize, mut value: u16, digits: usize) {
    let start = *at;
    *at += digits;
    for index in (start..*at).rev() {
        dst[index] = b'0' + (value % 10) as u8;
        value /= 10;
    }
}

impl core::fmt::Debug for FetchOrigin {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FetchOrigin")
            .field("scheme", &self.scheme)
            .field("hostname_len", &self.hostname_len)
            .field("port", &self.port)
            .finish()
    }
}

impl FetchOrigin {
    /// Parse a canonical manifest origin. Only lower-case ASCII DNS names are
    /// accepted; paths, queries, fragments, user-info, IP literals, and
    /// wildcards fail closed. Default ports must be omitted.
    pub fn parse(text: &str) -> Result<Self, OriginError> {
        let (scheme, rest) = if let Some(rest) = text.strip_prefix("https://") {
            (FetchScheme::Https, rest)
        } else if let Some(rest) = text.strip_prefix("http://") {
            (FetchScheme::Http, rest)
        } else if text.contains("://") {
            return Err(OriginError::UnsupportedScheme);
        } else {
            return Err(OriginError::Malformed);
        };
        if rest.is_empty() || rest.contains(['/', '?', '#']) {
            return Err(OriginError::Malformed);
        }
        if rest.contains('@') {
            return Err(OriginError::UserInfo);
        }
        if rest.contains('*') {
            return Err(OriginError::Wildcard);
        }
        if rest.starts_with('[') || rest.ends_with(']') {
            return Err(OriginError::InvalidHostname);
        }

        let (host, port) = match rest.rsplit_once(':') {
            Some((host, port)) => {
                if host.contains(':') || port.is_empty() {
                    return Err(OriginError::InvalidPort);
                }
                let port = parse_port(port)?;
                if port == scheme.default_port() {
                    return Err(OriginError::NonCanonical);
                }
                (host, port)
            }
            None => (rest, scheme.default_port()),
        };
        validate_hostname(host)?;
        let hostname_len = u8::try_from(host.len()).map_err(|_| OriginError::HostnameTooLong)?;
        let mut hostname = [0; MAX_FETCH_HOSTNAME_BYTES];
        hostname[..host.len()].copy_from_slice(host.as_bytes());
        Ok(Self {
            scheme,
            hostname,
            hostname_len,
            port,
        })
    }

    pub const fn scheme(&self) -> FetchScheme {
        self.scheme
    }

    pub fn hostname(&self) -> &str {
        core::str::from_utf8(&self.hostname[..usize::from(self.hostname_len)]).unwrap_or("")
    }

    pub const fn port(&self) -> u16 {
        self.port
    }
}

fn parse_port(text: &str) -> Result<u16, OriginError> {
    if text.starts_with('0') || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(OriginError::InvalidPort);
    }
    text.parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or(OriginError::InvalidPort)
}

fn validate_hostname(host: &str) -> Result<(), OriginError> {
    if host.is_empty() || host.len() > MAX_FETCH_HOSTNAME_BYTES {
        return Err(if host.is_empty() {
            OriginError::InvalidHostname
        } else {
            OriginError::HostnameTooLong
        });
    }
    if host.ends_with('.') || host.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(OriginError::NonCanonical);
    }
    if host
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return Err(OriginError::InvalidHostname);
    }
    for label in host.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(OriginError::InvalidHostname);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AllowlistError {
    TooMany,
    Duplicate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchAllowlist {
    origins: [Option<FetchOrigin>; MAX_FETCH_ORIGINS],
    len: u8,
}

impl FetchAllowlist {
    pub const fn empty() -> Self {
        Self {
            origins: [None; MAX_FETCH_ORIGINS],
            len: 0,
        }
    }

    pub fn push(&mut self, origin: FetchOrigin) -> Result<(), AllowlistError> {
        if self.contains(origin) {
            return Err(AllowlistError::Duplicate);
        }
        let index = usize::from(self.len);
        if index == MAX_FETCH_ORIGINS {
            return Err(AllowlistError::TooMany);
        }
        self.origins[index] = Some(origin);
        self.len += 1;
        Ok(())
    }

    pub fn contains(&self, origin: FetchOrigin) -> bool {
        self.origins[..usize::from(self.len)].contains(&Some(origin))
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

pub const MAX_SPKI_PINS_PER_ORIGIN: usize = 2;
pub const MAX_FETCH_CERTIFICATE_BYTES: usize = 8_192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpkiSha256([u8; 32]);

impl SpkiSha256 {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn parse_hex(text: &str) -> Result<Self, ManifestFetchError> {
        if text.len() != 64
            || text
                .bytes()
                .any(|byte| !matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(ManifestFetchError::InvalidPin);
        }
        let mut bytes = [0; 32];
        for (index, pair) in text.as_bytes().chunks_exact(2).enumerate() {
            bytes[index] = (hex_nibble(pair[0]).ok_or(ManifestFetchError::InvalidPin)? << 4)
                | hex_nibble(pair[1]).ok_or(ManifestFetchError::InvalidPin)?;
        }
        Ok(Self(bytes))
    }

    pub const fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchPinSet {
    pins: [Option<SpkiSha256>; MAX_SPKI_PINS_PER_ORIGIN],
    len: u8,
}

impl FetchPinSet {
    pub const fn empty() -> Self {
        Self {
            pins: [None; MAX_SPKI_PINS_PER_ORIGIN],
            len: 0,
        }
    }

    pub fn push(&mut self, pin: SpkiSha256) -> Result<(), ManifestFetchError> {
        if self.pins[..usize::from(self.len)].contains(&Some(pin))
            || usize::from(self.len) == MAX_SPKI_PINS_PER_ORIGIN
        {
            return Err(ManifestFetchError::InvalidPin);
        }
        self.pins[usize::from(self.len)] = Some(pin);
        self.len += 1;
        Ok(())
    }

    pub const fn len(&self) -> usize {
        self.len as usize
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn contains(&self, pin: SpkiSha256) -> bool {
        self.pins[..usize::from(self.len)].contains(&Some(pin))
    }

    /// Compare every configured digest without an early exit. The selected TLS
    /// backend remains responsible for hashing the exact SPKI DER returned by
    /// [`extract_certificate_spki_der`].
    pub fn matches_digest(&self, digest: &[u8; 32]) -> bool {
        let mut matched = 0u8;
        for slot in &self.pins {
            let mut different = 0u8;
            let (present, bytes) = match slot {
                Some(pin) => (1u8, pin.bytes()),
                None => (0u8, &[0; 32]),
            };
            for index in 0..32 {
                different |= bytes[index] ^ digest[index];
            }
            matched |= present & u8::from(different == 0);
        }
        matched != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchPinTable {
    sets: [FetchPinSet; MAX_FETCH_ORIGINS],
}

impl FetchPinTable {
    pub const fn empty() -> Self {
        Self {
            sets: [FetchPinSet::empty(); MAX_FETCH_ORIGINS],
        }
    }

    pub const fn get(&self, origin_index: usize) -> Option<&FetchPinSet> {
        if origin_index < MAX_FETCH_ORIGINS {
            Some(&self.sets[origin_index])
        } else {
            None
        }
    }

    pub fn complete_for(&self, allowlist: &FetchAllowlist) -> bool {
        (0..allowlist.len()).all(|index| {
            allowlist.origins[index].is_some_and(|origin| {
                origin.scheme() != FetchScheme::Https || !self.sets[index].is_empty()
            })
        })
    }
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpkiDerError {
    CertificateTooLarge,
    Malformed,
    UnsupportedKey,
}

/// Locate the complete DER `SubjectPublicKeyInfo` TLV in an X.509 certificate.
/// The caller retains the certificate bytes; this function allocates and copies
/// nothing. It accepts only definite, canonical DER lengths and the fixed X.509
/// TBSCertificate field order, leaving signature/path validation to TLS.
pub fn extract_certificate_spki_der(certificate: &[u8]) -> Result<&[u8], SpkiDerError> {
    if certificate.len() > MAX_FETCH_CERTIFICATE_BYTES {
        return Err(SpkiDerError::CertificateTooLarge);
    }
    let outer = der_take(certificate, 0x30)?;
    if !outer.rest.is_empty() {
        return Err(SpkiDerError::Malformed);
    }
    let tbs = der_take(outer.value, 0x30)?;
    let signature_algorithm = der_take(tbs.rest, 0x30)?;
    let signature = der_take(signature_algorithm.rest, 0x03)?;
    if signature.value.is_empty() || !signature.rest.is_empty() {
        return Err(SpkiDerError::Malformed);
    }

    let mut fields = tbs.value;
    if fields.first() == Some(&0xa0) {
        fields = der_take(fields, 0xa0)?.rest;
    }
    for tag in [0x02, 0x30, 0x30, 0x30, 0x30] {
        fields = der_take(fields, tag)?.rest;
    }
    let spki = der_take(fields, 0x30)?;
    let algorithm = der_take(spki.value, 0x30)?;
    let public_key = der_take(algorithm.rest, 0x03)?;
    if public_key.value.len() < 2 || public_key.value[0] != 0 || !public_key.rest.is_empty() {
        return Err(SpkiDerError::Malformed);
    }
    Ok(spki.full)
}

/// Extract an uncompressed SEC1 P-256 public key from an exact SPKI DER TLV.
/// Other algorithms, curves, compressed points, and trailing bytes fail
/// closed; the returned key borrows the caller-owned certificate storage.
pub fn extract_p256_public_key_from_spki_der(spki_der: &[u8]) -> Result<&[u8; 65], SpkiDerError> {
    const EC_P256_ALGORITHM: &[u8] = &[
        0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, // id-ecPublicKey
        0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, // prime256v1
    ];

    let spki = der_take(spki_der, 0x30)?;
    if !spki.rest.is_empty() {
        return Err(SpkiDerError::Malformed);
    }
    let algorithm = der_take(spki.value, 0x30)?;
    if algorithm.value != EC_P256_ALGORITHM {
        return Err(SpkiDerError::UnsupportedKey);
    }
    let public_key = der_take(algorithm.rest, 0x03)?;
    if !public_key.rest.is_empty()
        || public_key.value.len() != 66
        || public_key.value[0] != 0
        || public_key.value[1] != 0x04
    {
        return Err(SpkiDerError::UnsupportedKey);
    }
    public_key.value[1..]
        .try_into()
        .map_err(|_| SpkiDerError::Malformed)
}

/// Verify a TLS 1.3 server CertificateVerify signature for a pinned P-256 key
/// and an already-finalized SHA-256 handshake transcript.
#[cfg(feature = "app_fetch_tls_verifier")]
#[inline(never)]
pub fn verify_p256_tls13_certificate_signature(
    public_key: &[u8; 65],
    transcript_hash: &[u8; 32],
    signature_der: &[u8],
) -> bool {
    use p256::ecdsa::{signature::Verifier, Signature, VerifyingKey};

    const CONTEXT: &[u8] = b"TLS 1.3, server CertificateVerify\x00";
    let mut message = [0u8; 130];
    message[..64].fill(0x20);
    message[64..98].copy_from_slice(CONTEXT);
    message[98..].copy_from_slice(transcript_hash);
    let Ok(key) = VerifyingKey::from_sec1_bytes(public_key) else {
        return false;
    };
    let Ok(signature) = Signature::from_der(signature_der) else {
        return false;
    };
    key.verify(&message, &signature).is_ok()
}

struct DerTlv<'a> {
    full: &'a [u8],
    value: &'a [u8],
    rest: &'a [u8],
}

fn der_take(input: &[u8], tag: u8) -> Result<DerTlv<'_>, SpkiDerError> {
    if input.first() != Some(&tag) {
        return Err(SpkiDerError::Malformed);
    }
    let first_length = *input.get(1).ok_or(SpkiDerError::Malformed)?;
    let (header_len, value_len) = if first_length & 0x80 == 0 {
        (2usize, usize::from(first_length))
    } else {
        let octets = usize::from(first_length & 0x7f);
        if octets == 0 || octets > 2 || input.get(2) == Some(&0) {
            return Err(SpkiDerError::Malformed);
        }
        let mut length = 0usize;
        for byte in input.get(2..2 + octets).ok_or(SpkiDerError::Malformed)? {
            length = length
                .checked_mul(256)
                .and_then(|value| value.checked_add(usize::from(*byte)))
                .ok_or(SpkiDerError::Malformed)?;
        }
        if length < 128 {
            return Err(SpkiDerError::Malformed);
        }
        (2 + octets, length)
    };
    let total = header_len
        .checked_add(value_len)
        .ok_or(SpkiDerError::Malformed)?;
    let full = input.get(..total).ok_or(SpkiDerError::Malformed)?;
    let value = full.get(header_len..).ok_or(SpkiDerError::Malformed)?;
    Ok(DerTlv {
        full,
        value,
        rest: &input[total..],
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestFetchError {
    InvalidJson,
    InvalidShape,
    InvalidOrigin,
    InvalidPin,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestFetchPermission {
    pub legacy: Option<bool>,
    pub allowlist: FetchAllowlist,
    pub pins: FetchPinTable,
}

impl ManifestFetchPermission {
    /// No declaration: no legacy grant, empty allowlist, empty pin table.
    pub const fn empty() -> Self {
        Self {
            legacy: None,
            allowlist: FetchAllowlist::empty(),
            pins: FetchPinTable::empty(),
        }
    }
}

/// Parse only the root `permissions.network` member of a KPA manifest without
/// allocation. Unknown manifest members are structurally skipped with a fixed
/// nesting limit, so a nested attacker-controlled `network` key cannot be
/// mistaken for the root permission declaration.
pub fn parse_manifest_fetch_permission(
    manifest: &str,
    version: u32,
) -> Result<ManifestFetchPermission, ManifestFetchError> {
    let mut permission = ManifestFetchPermission::empty();
    parse_manifest_fetch_permission_into(manifest, version, &mut permission)?;
    Ok(permission)
}

/// [`parse_manifest_fetch_permission`] writing through `out` instead of
/// returning the ~1.3 KiB permission by value (KOTO-0252). The whole parse
/// chain builds origins and pin sets directly in the caller's storage, so no
/// permission-sized temporary lands on any frame between the manifest bytes
/// and the resident destination — on RP2040 the by-value pipeline stacked
/// several of these copies under the app-session poll frames. On `Err`, `out`
/// holds an unspecified partial value and must not be used.
pub fn parse_manifest_fetch_permission_into(
    manifest: &str,
    version: u32,
    out: &mut ManifestFetchPermission,
) -> Result<(), ManifestFetchError> {
    *out = ManifestFetchPermission::empty();
    let mut cursor = JsonCursor::new(manifest.as_bytes());
    cursor.ws();
    cursor.byte(b'{')?;
    let mut saw_permissions = false;
    cursor.ws();
    if cursor.take(b'}') {
        return cursor.finish();
    }
    loop {
        let key = cursor.string()?;
        if key.escaped {
            return Err(ManifestFetchError::InvalidJson);
        }
        cursor.ws();
        cursor.byte(b':')?;
        cursor.ws();
        if key.raw == b"permissions" {
            if saw_permissions {
                return Err(ManifestFetchError::InvalidShape);
            }
            saw_permissions = true;
            cursor.permissions_into(version, out)?;
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
    cursor.finish()
}

const MANIFEST_JSON_MAX_DEPTH: u8 = 8;

struct JsonString<'a> {
    raw: &'a [u8],
    escaped: bool,
}

struct JsonCursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> JsonCursor<'a> {
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

    fn byte(&mut self, expected: u8) -> Result<(), ManifestFetchError> {
        self.take(expected)
            .then_some(())
            .ok_or(ManifestFetchError::InvalidJson)
    }

    fn finish(&mut self) -> Result<(), ManifestFetchError> {
        self.ws();
        if self.at == self.bytes.len() {
            Ok(())
        } else {
            Err(ManifestFetchError::InvalidJson)
        }
    }

    fn string(&mut self) -> Result<JsonString<'a>, ManifestFetchError> {
        self.byte(b'"')?;
        let start = self.at;
        let mut escaped = false;
        while let Some(&byte) = self.bytes.get(self.at) {
            match byte {
                b'"' => {
                    let raw = &self.bytes[start..self.at];
                    self.at += 1;
                    return Ok(JsonString { raw, escaped });
                }
                b'\\' => {
                    escaped = true;
                    self.at += 1;
                    let escape = *self
                        .bytes
                        .get(self.at)
                        .ok_or(ManifestFetchError::InvalidJson)?;
                    match escape {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                            self.at += 1;
                        }
                        b'u' => {
                            self.at += 1;
                            for _ in 0..4 {
                                let digit = *self
                                    .bytes
                                    .get(self.at)
                                    .ok_or(ManifestFetchError::InvalidJson)?;
                                if !digit.is_ascii_hexdigit() {
                                    return Err(ManifestFetchError::InvalidJson);
                                }
                                self.at += 1;
                            }
                        }
                        _ => return Err(ManifestFetchError::InvalidJson),
                    }
                }
                0x00..=0x1f => return Err(ManifestFetchError::InvalidJson),
                _ => self.at += 1,
            }
        }
        Err(ManifestFetchError::InvalidJson)
    }

    // KOTO-0252: the permissions/network/origins walkers write through `out`
    // references. Returning `ManifestFetchPermission` / the allowlist+pins
    // tuple by value stacked ~1.3 KiB temporaries on every frame of this
    // chain, which sits under the device app-launch path.
    fn permissions_into(
        &mut self,
        version: u32,
        out: &mut ManifestFetchPermission,
    ) -> Result<(), ManifestFetchError> {
        self.byte(b'{')?;
        let mut saw_network = false;
        self.ws();
        if self.take(b'}') {
            return Ok(());
        }
        loop {
            let key = self.string()?;
            if key.escaped {
                return Err(ManifestFetchError::InvalidJson);
            }
            self.ws();
            self.byte(b':')?;
            self.ws();
            if key.raw == b"network" {
                if saw_network {
                    return Err(ManifestFetchError::InvalidShape);
                }
                saw_network = true;
                if version == 1 {
                    out.legacy = Some(self.boolean()?);
                } else if version == 2 {
                    self.network_v2_into(&mut out.allowlist, &mut out.pins)?;
                } else {
                    return Err(ManifestFetchError::InvalidShape);
                }
            } else {
                self.value(2)?;
            }
            self.ws();
            if self.take(b'}') {
                return Ok(());
            }
            self.byte(b',')?;
            self.ws();
        }
    }

    fn network_v2_into(
        &mut self,
        allowlist: &mut FetchAllowlist,
        pins: &mut FetchPinTable,
    ) -> Result<(), ManifestFetchError> {
        self.byte(b'{')?;
        self.ws();
        let key = self.string()?;
        if key.raw != b"origins" || key.escaped {
            return Err(ManifestFetchError::InvalidShape);
        }
        self.ws();
        self.byte(b':')?;
        self.ws();
        self.origins_into(allowlist, pins)?;
        self.ws();
        self.byte(b'}')?;
        Ok(())
    }

    fn origins_into(
        &mut self,
        allowlist: &mut FetchAllowlist,
        pins: &mut FetchPinTable,
    ) -> Result<(), ManifestFetchError> {
        self.byte(b'[')?;
        *allowlist = FetchAllowlist::empty();
        *pins = FetchPinTable::empty();
        self.ws();
        if self.take(b']') {
            return Ok(());
        }
        loop {
            let (origin, pin_set) = if self.bytes.get(self.at) == Some(&b'{') {
                self.pinned_origin()?
            } else {
                let value = self.string()?;
                if value.escaped {
                    return Err(ManifestFetchError::InvalidOrigin);
                }
                let text = core::str::from_utf8(value.raw)
                    .map_err(|_| ManifestFetchError::InvalidOrigin)?;
                (
                    FetchOrigin::parse(text).map_err(|_| ManifestFetchError::InvalidOrigin)?,
                    FetchPinSet::empty(),
                )
            };
            let index = allowlist.len();
            allowlist
                .push(origin)
                .map_err(|_| ManifestFetchError::InvalidOrigin)?;
            pins.sets[index] = pin_set;
            self.ws();
            if self.take(b']') {
                return Ok(());
            }
            self.byte(b',')?;
            self.ws();
        }
    }

    fn pinned_origin(&mut self) -> Result<(FetchOrigin, FetchPinSet), ManifestFetchError> {
        self.byte(b'{')?;
        let mut origin = None;
        let mut pins = None;
        self.ws();
        if self.take(b'}') {
            return Err(ManifestFetchError::InvalidShape);
        }
        loop {
            let key = self.string()?;
            if key.escaped {
                return Err(ManifestFetchError::InvalidJson);
            }
            self.ws();
            self.byte(b':')?;
            self.ws();
            match key.raw {
                b"origin" if origin.is_none() => {
                    let value = self.string()?;
                    if value.escaped {
                        return Err(ManifestFetchError::InvalidOrigin);
                    }
                    let text = core::str::from_utf8(value.raw)
                        .map_err(|_| ManifestFetchError::InvalidOrigin)?;
                    origin = Some(
                        FetchOrigin::parse(text).map_err(|_| ManifestFetchError::InvalidOrigin)?,
                    );
                }
                b"spki_sha256" if pins.is_none() => pins = Some(self.pin_set()?),
                _ => return Err(ManifestFetchError::InvalidShape),
            }
            self.ws();
            if self.take(b'}') {
                break;
            }
            self.byte(b',')?;
            self.ws();
        }
        let origin = origin.ok_or(ManifestFetchError::InvalidShape)?;
        let pins = pins.ok_or(ManifestFetchError::InvalidShape)?;
        if origin.scheme() != FetchScheme::Https || pins.is_empty() {
            return Err(ManifestFetchError::InvalidPin);
        }
        Ok((origin, pins))
    }

    fn pin_set(&mut self) -> Result<FetchPinSet, ManifestFetchError> {
        self.byte(b'[')?;
        let mut result = FetchPinSet::empty();
        self.ws();
        if self.take(b']') {
            return Err(ManifestFetchError::InvalidPin);
        }
        loop {
            let value = self.string()?;
            if value.escaped {
                return Err(ManifestFetchError::InvalidPin);
            }
            let text =
                core::str::from_utf8(value.raw).map_err(|_| ManifestFetchError::InvalidPin)?;
            result.push(SpkiSha256::parse_hex(text)?)?;
            self.ws();
            if self.take(b']') {
                return Ok(result);
            }
            self.byte(b',')?;
            self.ws();
        }
    }

    fn boolean(&mut self) -> Result<bool, ManifestFetchError> {
        if self.literal(b"true") {
            Ok(true)
        } else if self.literal(b"false") {
            Ok(false)
        } else {
            Err(ManifestFetchError::InvalidShape)
        }
    }

    fn literal(&mut self, literal: &[u8]) -> bool {
        if self.bytes.get(self.at..self.at + literal.len()) == Some(literal) {
            self.at += literal.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: u8) -> Result<(), ManifestFetchError> {
        if depth > MANIFEST_JSON_MAX_DEPTH {
            return Err(ManifestFetchError::InvalidJson);
        }
        self.ws();
        match self.bytes.get(self.at).copied() {
            Some(b'"') => self.string().map(|_| ()),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b't') if self.literal(b"true") => Ok(()),
            Some(b'f') if self.literal(b"false") => Ok(()),
            Some(b'n') if self.literal(b"null") => Ok(()),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(ManifestFetchError::InvalidJson),
        }
    }

    fn object(&mut self, depth: u8) -> Result<(), ManifestFetchError> {
        self.byte(b'{')?;
        self.ws();
        if self.take(b'}') {
            return Ok(());
        }
        loop {
            if self.string()?.escaped {
                return Err(ManifestFetchError::InvalidJson);
            }
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

    fn array(&mut self, depth: u8) -> Result<(), ManifestFetchError> {
        self.byte(b'[')?;
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

    fn number(&mut self) -> Result<(), ManifestFetchError> {
        let start = self.at;
        self.take(b'-');
        if self.take(b'0') {
            if self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                return Err(ManifestFetchError::InvalidJson);
            }
        } else {
            let digit_start = self.at;
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
            if self.at == digit_start {
                return Err(ManifestFetchError::InvalidJson);
            }
        }
        if self.take(b'.') {
            let digits = self.at;
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
            if self.at == digits {
                return Err(ManifestFetchError::InvalidJson);
            }
        }
        if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
            self.at += 1;
            if matches!(self.bytes.get(self.at), Some(b'+' | b'-')) {
                self.at += 1;
            }
            let digits = self.at;
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
            if self.at == digits {
                return Err(ManifestFetchError::InvalidJson);
            }
        }
        (self.at > start)
            .then_some(())
            .ok_or(ManifestFetchError::InvalidJson)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppContext {
    pub app_id: u32,
    pub generation: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchRequestId(u32);

impl FetchRequestId {
    /// Reconstruct an untrusted VM-supplied ID. `AppFetchController` still
    /// checks its generation, live slot, and implicit owner before every
    /// operation.
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
pub enum FetchError {
    Denied = 1,
    Unavailable = 2,
    Busy = 3,
    MalformedUrl = 4,
    Timeout = 5,
    Cancelled = 6,
    Dns = 7,
    ForbiddenAddress = 8,
    Connect = 9,
    Tls = 10,
    Protocol = 11,
    ResponseTooLarge = 12,
    Disconnected = 13,
    StaleRequest = 14,
    ForeignRequest = 15,
    BufferTooLarge = 16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FetchPoll {
    Pending,
    Headers { status: u16 },
    Body,
    Complete,
    Failed(FetchError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchDiagnostics {
    pub state: FetchPoll,
    pub origin_index: u8,
    pub request_generation: u16,
    pub bytes_read: u32,
    pub elapsed_ms: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendPoll {
    Pending,
    Headers { status: u16 },
    Body,
    Complete,
    Failed(FetchError),
}

/// Ownership state for the single OS-private Fetch transport exchange. Only
/// copied URL/body bytes cross this boundary; sockets and TLS objects remain
/// owned by the network executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FetchTransportState {
    Idle,
    Queued,
    Running,
    Headers,
    Body,
    Complete,
    Failed,
    CancelRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FetchTransportCommand {
    pub request: FetchRequestId,
    pub url_len: u16,
    pub pins: FetchPinSet,
}

/// Fixed-capacity producer/consumer contract between `DeviceHost` and the
/// network future. It deliberately supports one live secure transport: RP2040
/// lends the audio PCM workspace to exactly one TLS future, and RP2350A uses
/// the same observable semantics.
pub struct FetchTransportMailbox {
    state: FetchTransportState,
    request: FetchRequestId,
    url: [u8; MAX_FETCH_URL_BYTES],
    url_len: u16,
    pins: FetchPinSet,
    status: u16,
    body: [u8; FETCH_TRANSPORT_CHUNK_BYTES],
    body_len: u16,
    body_at: u16,
    error: FetchError,
    headers_polled: bool,
}

impl FetchTransportMailbox {
    pub const fn new() -> Self {
        Self {
            state: FetchTransportState::Idle,
            request: FetchRequestId::from_raw(0),
            url: [0; MAX_FETCH_URL_BYTES],
            url_len: 0,
            pins: FetchPinSet::empty(),
            status: 0,
            body: [0; FETCH_TRANSPORT_CHUNK_BYTES],
            body_len: 0,
            body_at: 0,
            error: FetchError::Unavailable,
            headers_polled: false,
        }
    }

    pub const fn state(&self) -> FetchTransportState {
        self.state
    }

    pub const fn active_request(&self) -> Option<FetchRequestId> {
        if matches!(self.state, FetchTransportState::Idle) {
            None
        } else {
            Some(self.request)
        }
    }

    pub fn submit(
        &mut self,
        request: FetchRequestId,
        url: &str,
        pins: FetchPinSet,
    ) -> Result<(), FetchError> {
        if self.state != FetchTransportState::Idle {
            return Err(FetchError::Busy);
        }
        if url.len() > self.url.len() {
            return Err(FetchError::MalformedUrl);
        }
        self.clear_payloads();
        self.request = request;
        self.url[..url.len()].copy_from_slice(url.as_bytes());
        self.url_len = url.len() as u16;
        self.pins = pins;
        self.state = FetchTransportState::Queued;
        Ok(())
    }

    /// Copies a queued command into executor-owned storage and transfers it to
    /// `Running`. The executor may then retain the copy across async awaits.
    pub fn take_command(
        &mut self,
        dst: &mut [u8],
    ) -> Result<Option<FetchTransportCommand>, FetchError> {
        if self.state != FetchTransportState::Queued {
            return Ok(None);
        }
        let len = usize::from(self.url_len);
        if dst.len() < len {
            return Err(FetchError::BufferTooLarge);
        }
        dst[..len].copy_from_slice(&self.url[..len]);
        self.state = FetchTransportState::Running;
        Ok(Some(FetchTransportCommand {
            request: self.request,
            url_len: self.url_len,
            pins: self.pins,
        }))
    }

    pub fn publish_headers(
        &mut self,
        request: FetchRequestId,
        status: u16,
    ) -> Result<(), FetchError> {
        self.require_running(request)?;
        self.status = status;
        self.state = FetchTransportState::Headers;
        Ok(())
    }

    /// Publishes at most one VM-readable chunk. The producer must wait until
    /// `read_body` drains it before reusing the slot.
    pub fn publish_body(&mut self, request: FetchRequestId, src: &[u8]) -> Result<(), FetchError> {
        if src.len() > self.body.len() {
            return Err(FetchError::BufferTooLarge);
        }
        if self.request != request {
            return Err(FetchError::StaleRequest);
        }
        if !matches!(
            self.state,
            FetchTransportState::Running | FetchTransportState::Headers
        ) {
            return Err(FetchError::Busy);
        }
        self.body[..src.len()].copy_from_slice(src);
        self.body_len = src.len() as u16;
        self.body_at = 0;
        self.state = FetchTransportState::Body;
        Ok(())
    }

    pub fn read_body(
        &mut self,
        request: FetchRequestId,
        dst: &mut [u8],
    ) -> Result<usize, FetchError> {
        if self.request != request || self.state == FetchTransportState::Idle {
            return Err(FetchError::StaleRequest);
        }
        if self.state == FetchTransportState::Failed {
            return Err(self.error);
        }
        if self.state != FetchTransportState::Body {
            return Ok(0);
        }
        let at = usize::from(self.body_at);
        let len = usize::from(self.body_len);
        let copied = dst.len().min(len.saturating_sub(at));
        dst[..copied].copy_from_slice(&self.body[at..at + copied]);
        self.body_at = self.body_at.saturating_add(copied as u16);
        if usize::from(self.body_at) == len {
            self.body[..len].fill(0);
            self.body_len = 0;
            self.body_at = 0;
            self.state = FetchTransportState::Running;
        }
        Ok(copied)
    }

    pub fn complete(&mut self, request: FetchRequestId) -> Result<(), FetchError> {
        self.require_running(request)?;
        self.state = FetchTransportState::Complete;
        Ok(())
    }

    pub fn fail(&mut self, request: FetchRequestId, error: FetchError) -> Result<(), FetchError> {
        if self.request != request || self.state == FetchTransportState::Idle {
            return Err(FetchError::StaleRequest);
        }
        self.error = error;
        self.state = FetchTransportState::Failed;
        Ok(())
    }

    /// Consumer-side poll that also records the sim-parity guarantee: once the
    /// VM has observed `Headers { status }`, the producer may supersede the
    /// Headers state with the first body chunk. The producer waits on
    /// [`Self::headers_polled`] so device apps never lose success metadata.
    pub fn poll_mut(&mut self, request: FetchRequestId) -> BackendPoll {
        let result = self.poll(request);
        if matches!(result, BackendPoll::Headers { .. }) {
            self.headers_polled = true;
        }
        result
    }

    pub const fn headers_polled(&self) -> bool {
        self.headers_polled
    }

    pub fn poll(&self, request: FetchRequestId) -> BackendPoll {
        if self.request != request || self.state == FetchTransportState::Idle {
            return BackendPoll::Failed(FetchError::StaleRequest);
        }
        match self.state {
            FetchTransportState::Idle => BackendPoll::Failed(FetchError::StaleRequest),
            FetchTransportState::Queued
            | FetchTransportState::Running
            | FetchTransportState::CancelRequested => BackendPoll::Pending,
            FetchTransportState::Headers => BackendPoll::Headers {
                status: self.status,
            },
            FetchTransportState::Body => BackendPoll::Body,
            FetchTransportState::Complete => BackendPoll::Complete,
            FetchTransportState::Failed => BackendPoll::Failed(self.error),
        }
    }

    pub fn request_cancel(&mut self, request: FetchRequestId) -> Result<(), FetchError> {
        if self.request != request || self.state == FetchTransportState::Idle {
            return Err(FetchError::StaleRequest);
        }
        self.state = FetchTransportState::CancelRequested;
        Ok(())
    }

    pub const fn cancel_requested(&self, request: FetchRequestId) -> bool {
        self.request.raw() == request.raw()
            && matches!(self.state, FetchTransportState::CancelRequested)
    }

    /// Executor acknowledgement after its socket/TLS future has been dropped.
    pub fn acknowledge_cancel(&mut self, request: FetchRequestId) -> Result<(), FetchError> {
        if !self.cancel_requested(request) {
            return Err(FetchError::StaleRequest);
        }
        self.reset();
        Ok(())
    }

    pub fn release_terminal(&mut self, request: FetchRequestId) -> Result<(), FetchError> {
        if self.request != request
            || !matches!(
                self.state,
                FetchTransportState::Complete | FetchTransportState::Failed
            )
        {
            return Err(FetchError::StaleRequest);
        }
        self.reset();
        Ok(())
    }

    fn require_running(&self, request: FetchRequestId) -> Result<(), FetchError> {
        if self.request != request {
            return Err(FetchError::StaleRequest);
        }
        if !matches!(
            self.state,
            FetchTransportState::Running | FetchTransportState::Headers
        ) {
            return Err(FetchError::Busy);
        }
        Ok(())
    }

    fn reset(&mut self) {
        self.clear_payloads();
        self.request = FetchRequestId::from_raw(0);
        self.pins = FetchPinSet::empty();
        self.status = 0;
        self.error = FetchError::Unavailable;
        self.state = FetchTransportState::Idle;
    }

    fn clear_payloads(&mut self) {
        self.url.fill(0);
        self.url_len = 0;
        self.body.fill(0);
        self.body_len = 0;
        self.body_at = 0;
        self.headers_polled = false;
    }
}

impl Default for FetchTransportMailbox {
    fn default() -> Self {
        Self::new()
    }
}

/// OS-private transport seam. Implementations retain every network-stack
/// object and copy at most `dst.len()` bytes into caller-owned storage.
pub trait FetchBackend {
    fn available(&self) -> bool;
    fn start(
        &mut self,
        request: FetchRequestId,
        url: &str,
        pins: FetchPinSet,
    ) -> Result<(), FetchError>;
    fn poll(&mut self, request: FetchRequestId) -> BackendPoll;
    fn read(&mut self, request: FetchRequestId, dst: &mut [u8]) -> Result<usize, FetchError>;
    fn cancel(&mut self, request: FetchRequestId);
}

/// Zero-sized backend for device/offline profiles that deliberately link no
/// DNS, socket, TLS, timer, buffer, or retry implementation.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UnavailableFetchBackend;

impl FetchBackend for UnavailableFetchBackend {
    fn available(&self) -> bool {
        false
    }

    fn start(&mut self, _: FetchRequestId, _: &str, _: FetchPinSet) -> Result<(), FetchError> {
        Err(FetchError::Unavailable)
    }

    fn poll(&mut self, _: FetchRequestId) -> BackendPoll {
        BackendPoll::Failed(FetchError::Unavailable)
    }

    fn read(&mut self, _: FetchRequestId, _: &mut [u8]) -> Result<usize, FetchError> {
        Err(FetchError::Unavailable)
    }

    fn cancel(&mut self, _: FetchRequestId) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RequestSlot {
    id: FetchRequestId,
    owner: AppContext,
    origin_index: u8,
    started_ms: u64,
    bytes_read: u32,
    state: FetchPoll,
}

#[derive(Debug)]
pub struct AppFetchController {
    generation: u16,
    sequence: u16,
    slots: [Option<RequestSlot>; MAX_GLOBAL_FETCH_REQUESTS],
}

impl AppFetchController {
    pub const fn new() -> Self {
        Self {
            generation: 1,
            sequence: 0,
            slots: [None; MAX_GLOBAL_FETCH_REQUESTS],
        }
    }

    pub fn start<B: FetchBackend>(
        &mut self,
        backend: &mut B,
        app: AppContext,
        allowlist: &FetchAllowlist,
        url: &str,
        now_ms: u64,
    ) -> Result<FetchRequestId, FetchError> {
        self.start_inner(backend, app, allowlist, None, url, now_ms)
    }

    pub fn start_pinned<B: FetchBackend>(
        &mut self,
        backend: &mut B,
        app: AppContext,
        allowlist: &FetchAllowlist,
        pins: &FetchPinTable,
        url: &str,
        now_ms: u64,
    ) -> Result<FetchRequestId, FetchError> {
        self.start_inner(backend, app, allowlist, Some(pins), url, now_ms)
    }

    fn start_inner<B: FetchBackend>(
        &mut self,
        backend: &mut B,
        app: AppContext,
        allowlist: &FetchAllowlist,
        pins: Option<&FetchPinTable>,
        url: &str,
        now_ms: u64,
    ) -> Result<FetchRequestId, FetchError> {
        if !backend.available() {
            return Err(FetchError::Unavailable);
        }
        if url.len() > MAX_FETCH_URL_BYTES {
            return Err(FetchError::MalformedUrl);
        }
        if self.slots.iter().flatten().any(|slot| slot.owner == app) {
            return Err(FetchError::Busy);
        }
        let origin = parse_fetch_url(url)
            .map_err(|_| FetchError::MalformedUrl)?
            .origin();
        let origin_index = allowlist.origins[..allowlist.len()]
            .iter()
            .position(|entry| *entry == Some(origin))
            .ok_or(FetchError::Denied)? as u8;
        let slot_index = self
            .slots
            .iter()
            .position(Option::is_none)
            .ok_or(FetchError::Busy)?;
        self.sequence = self.sequence.wrapping_add(1);
        if self.sequence == 0 {
            self.sequence = 1;
        }
        let id = FetchRequestId::new(self.generation, self.sequence);
        let selected_pins = pins
            .and_then(|table| table.get(origin_index as usize))
            .copied()
            .unwrap_or_else(FetchPinSet::empty);
        backend.start(id, url, selected_pins)?;
        self.slots[slot_index] = Some(RequestSlot {
            id,
            owner: app,
            origin_index,
            started_ms: now_ms,
            bytes_read: 0,
            state: FetchPoll::Pending,
        });
        Ok(id)
    }

    pub fn poll<B: FetchBackend>(
        &mut self,
        backend: &mut B,
        app: AppContext,
        id: FetchRequestId,
        now_ms: u64,
    ) -> Result<FetchPoll, FetchError> {
        let index = self.slot_index(app, id)?;
        let slot = self.slots[index].as_mut().expect("located slot");
        if now_ms.saturating_sub(slot.started_ms) > u64::from(MAX_FETCH_DURATION_MS) {
            backend.cancel(id);
            slot.state = FetchPoll::Failed(FetchError::Timeout);
            return Ok(slot.state);
        }
        slot.state = match backend.poll(id) {
            BackendPoll::Pending => FetchPoll::Pending,
            BackendPoll::Headers { status } => FetchPoll::Headers { status },
            BackendPoll::Body => FetchPoll::Body,
            BackendPoll::Complete => FetchPoll::Complete,
            BackendPoll::Failed(error) => FetchPoll::Failed(error),
        };
        Ok(slot.state)
    }

    pub fn read<B: FetchBackend>(
        &mut self,
        backend: &mut B,
        app: AppContext,
        id: FetchRequestId,
        dst: &mut [u8],
    ) -> Result<usize, FetchError> {
        if dst.len() > MAX_FETCH_READ_BYTES {
            return Err(FetchError::BufferTooLarge);
        }
        let index = self.slot_index(app, id)?;
        let slot = self.slots[index].as_mut().expect("located slot");
        let remaining = MAX_FETCH_TOTAL_BYTES.saturating_sub(slot.bytes_read);
        if remaining == 0 {
            backend.cancel(id);
            slot.state = FetchPoll::Failed(FetchError::ResponseTooLarge);
            return Err(FetchError::ResponseTooLarge);
        }
        let cap = dst.len().min(remaining as usize);
        let read = backend.read(id, &mut dst[..cap])?;
        slot.bytes_read = slot.bytes_read.saturating_add(read as u32);
        Ok(read)
    }

    pub fn cancel<B: FetchBackend>(
        &mut self,
        backend: &mut B,
        app: AppContext,
        id: FetchRequestId,
    ) -> Result<(), FetchError> {
        let index = self.slot_index(app, id)?;
        backend.cancel(id);
        self.slots[index] = None;
        Ok(())
    }

    pub fn diagnostics(
        &self,
        app: AppContext,
        id: FetchRequestId,
        now_ms: u64,
    ) -> Result<FetchDiagnostics, FetchError> {
        let slot = &self.slots[self.slot_index(app, id)?].expect("located slot");
        Ok(FetchDiagnostics {
            state: slot.state,
            origin_index: slot.origin_index,
            request_generation: self.generation,
            bytes_read: slot.bytes_read,
            elapsed_ms: now_ms
                .saturating_sub(slot.started_ms)
                .min(u64::from(u32::MAX)) as u32,
        })
    }

    /// Cancel all state when the NetworkService generation, capability, or
    /// active application changes. Advancing the generation invalidates every
    /// previously issued request ID.
    pub fn teardown<B: FetchBackend>(&mut self, backend: &mut B) {
        for slot in self.slots.iter_mut() {
            if let Some(request) = slot.take() {
                backend.cancel(request.id);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        if self.generation == 0 {
            self.generation = 1;
        }
    }

    /// Reinitialize an OS-owned service object at a persisted lifecycle epoch.
    /// This is intended for control planes reconstructed from overlaid storage:
    /// the caller retains the epoch outside that storage so request IDs cannot
    /// collide across app launches.
    pub fn reset_generation<B: FetchBackend>(&mut self, backend: &mut B, generation: u16) {
        for slot in self.slots.iter_mut() {
            if let Some(request) = slot.take() {
                backend.cancel(request.id);
            }
        }
        self.generation = generation.max(1);
        self.sequence = 0;
    }

    fn slot_index(&self, app: AppContext, id: FetchRequestId) -> Result<usize, FetchError> {
        if (id.raw() >> 16) as u16 != self.generation {
            return Err(FetchError::StaleRequest);
        }
        let index = self
            .slots
            .iter()
            .position(|slot| slot.is_some_and(|slot| slot.id == id))
            .ok_or(FetchError::StaleRequest)?;
        if self.slots[index].expect("located slot").owner != app {
            return Err(FetchError::ForeignRequest);
        }
        Ok(index)
    }
}

impl Default for AppFetchController {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience owner used by KotoSim and ordinary backends. Device firmware
/// may instead retain [`AppFetchController`] and borrow its arena-owned backend
/// for each bounded host call.
#[derive(Debug)]
pub struct AppFetchService<B: FetchBackend> {
    backend: B,
    controller: AppFetchController,
}

impl<B: FetchBackend> AppFetchService<B> {
    pub const fn new(backend: B) -> Self {
        Self {
            backend,
            controller: AppFetchController::new(),
        }
    }

    pub fn start(
        &mut self,
        app: AppContext,
        allowlist: &FetchAllowlist,
        url: &str,
        now_ms: u64,
    ) -> Result<FetchRequestId, FetchError> {
        self.controller
            .start(&mut self.backend, app, allowlist, url, now_ms)
    }

    pub fn poll(
        &mut self,
        app: AppContext,
        id: FetchRequestId,
        now_ms: u64,
    ) -> Result<FetchPoll, FetchError> {
        self.controller.poll(&mut self.backend, app, id, now_ms)
    }

    pub fn read(
        &mut self,
        app: AppContext,
        id: FetchRequestId,
        dst: &mut [u8],
    ) -> Result<usize, FetchError> {
        self.controller.read(&mut self.backend, app, id, dst)
    }

    pub fn cancel(&mut self, app: AppContext, id: FetchRequestId) -> Result<(), FetchError> {
        self.controller.cancel(&mut self.backend, app, id)
    }

    pub fn diagnostics(
        &self,
        app: AppContext,
        id: FetchRequestId,
        now_ms: u64,
    ) -> Result<FetchDiagnostics, FetchError> {
        self.controller.diagnostics(app, id, now_ms)
    }

    pub fn teardown(&mut self) {
        self.controller.teardown(&mut self.backend);
    }

    pub fn reset_generation(&mut self, generation: u16) {
        self.controller
            .reset_generation(&mut self.backend, generation);
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HttpDecodeState {
    Headers,
    Body,
    Complete,
    Failed(FetchError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpDecodeProgress {
    pub consumed: usize,
    pub written: usize,
    pub state: HttpDecodeState,
    pub status: Option<u16>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HttpBodyMode {
    Undecided,
    ContentLength { remaining: u32 },
    Chunked,
    Empty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChunkState {
    Size,
    SizeLf { size: u32 },
    Data { remaining: u32 },
    DataCr,
    DataLf,
    FinalCr,
    FinalEndLf,
}

/// Allocation-free HTTP/1.1 response decoder. Callers retain any input not
/// reported as consumed and provide caller-owned output storage on each call.
/// V1 accepts only Content-Length or chunked framing and rejects trailers.
pub struct HttpResponseDecoder {
    headers: [u8; MAX_FETCH_HEADER_BYTES],
    header_len: usize,
    state: HttpDecodeState,
    status: Option<u16>,
    body_mode: HttpBodyMode,
    chunk_state: ChunkState,
    chunk_size: [u8; 16],
    chunk_size_len: usize,
    body_bytes: u32,
}

impl HttpResponseDecoder {
    pub const fn new() -> Self {
        Self {
            headers: [0; MAX_FETCH_HEADER_BYTES],
            header_len: 0,
            state: HttpDecodeState::Headers,
            status: None,
            body_mode: HttpBodyMode::Undecided,
            chunk_state: ChunkState::Size,
            chunk_size: [0; 16],
            chunk_size_len: 0,
            body_bytes: 0,
        }
    }

    pub const fn state(&self) -> HttpDecodeState {
        self.state
    }

    pub const fn status(&self) -> Option<u16> {
        self.status
    }

    pub const fn body_bytes(&self) -> u32 {
        self.body_bytes
    }

    pub fn reset(&mut self) {
        self.headers.fill(0);
        self.chunk_size.fill(0);
        *self = Self::new();
    }

    pub fn push(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<HttpDecodeProgress, FetchError> {
        if let HttpDecodeState::Failed(error) = self.state {
            return Err(error);
        }
        let mut consumed = 0usize;
        let mut written = 0usize;

        if self.state == HttpDecodeState::Headers {
            while consumed < input.len() {
                if self.header_len == MAX_FETCH_HEADER_BYTES {
                    return self.fail(FetchError::ResponseTooLarge);
                }
                self.headers[self.header_len] = input[consumed];
                self.header_len += 1;
                consumed += 1;
                if self.header_len >= 4
                    && self.headers[self.header_len - 4..self.header_len] == *b"\r\n\r\n"
                {
                    if let Err(error) = self.parse_headers() {
                        if self.state != HttpDecodeState::Failed(error) {
                            return self.fail(error);
                        }
                        return Err(error);
                    }
                    break;
                }
            }
        }

        if self.state == HttpDecodeState::Body && consumed < input.len() {
            match self.body_mode {
                HttpBodyMode::ContentLength { remaining } => {
                    let count = (remaining as usize)
                        .min(input.len() - consumed)
                        .min(output.len());
                    output[..count].copy_from_slice(&input[consumed..consumed + count]);
                    consumed += count;
                    written += count;
                    self.body_bytes = self.body_bytes.saturating_add(count as u32);
                    let remaining = remaining - count as u32;
                    self.body_mode = HttpBodyMode::ContentLength { remaining };
                    if remaining == 0 {
                        self.state = HttpDecodeState::Complete;
                    }
                }
                HttpBodyMode::Chunked => {
                    let progress = match self.decode_chunked(&input[consumed..], output) {
                        Ok(progress) => progress,
                        Err(error) => {
                            if self.state != HttpDecodeState::Failed(error) {
                                return self.fail(error);
                            }
                            return Err(error);
                        }
                    };
                    consumed += progress.0;
                    written += progress.1;
                }
                HttpBodyMode::Empty => self.state = HttpDecodeState::Complete,
                HttpBodyMode::Undecided => return self.fail(FetchError::Protocol),
            }
        }

        Ok(HttpDecodeProgress {
            consumed,
            written,
            state: self.state,
            status: self.status,
        })
    }

    fn parse_headers(&mut self) -> Result<(), FetchError> {
        let headers = &self.headers[..self.header_len - 2];
        let mut cursor = 0usize;
        let status_line = next_crlf_line(headers, &mut cursor).ok_or(FetchError::Protocol)?;
        if status_line.len() < 12 || !status_line.starts_with(b"HTTP/1.1 ") {
            return self.fail(FetchError::Protocol);
        }
        let code = parse_three_digits(&status_line[9..12]).ok_or(FetchError::Protocol)?;
        if status_line.len() > 12 && status_line[12] != b' ' {
            return self.fail(FetchError::Protocol);
        }
        if (100..200).contains(&code) || (300..400).contains(&code) {
            return self.fail(FetchError::Protocol);
        }
        self.status = Some(code);

        let mut content_length = None;
        let mut chunked = false;
        while let Some(line) = next_crlf_line(headers, &mut cursor) {
            if line.is_empty() {
                break;
            }
            if line[0] == b' ' || line[0] == b'\t' {
                return self.fail(FetchError::Protocol);
            }
            let colon = line
                .iter()
                .position(|byte| *byte == b':')
                .ok_or(FetchError::Protocol)?;
            let name = &line[..colon];
            let value = trim_ascii_space(&line[colon + 1..]);
            if name.is_empty()
                || !name.iter().all(|byte| is_header_name_byte(*byte))
                || !value
                    .iter()
                    .all(|byte| *byte == b'\t' || (0x20..=0x7e).contains(byte))
            {
                return self.fail(FetchError::Protocol);
            }
            if ascii_eq_ignore_case(name, b"content-length") {
                let parsed = parse_decimal_u32(value).ok_or(FetchError::Protocol)?;
                if content_length.is_some_and(|prior| prior != parsed) {
                    return self.fail(FetchError::Protocol);
                }
                content_length = Some(parsed);
            } else if ascii_eq_ignore_case(name, b"transfer-encoding") {
                if chunked || !ascii_eq_ignore_case(value, b"chunked") {
                    return self.fail(FetchError::Protocol);
                }
                chunked = true;
            }
        }

        if chunked && content_length.is_some() {
            return self.fail(FetchError::Protocol);
        }
        if code == 204 || code == 304 {
            if chunked || content_length.is_some_and(|length| length != 0) {
                return self.fail(FetchError::Protocol);
            }
            self.body_mode = HttpBodyMode::Empty;
            self.state = HttpDecodeState::Complete;
        } else if chunked {
            self.body_mode = HttpBodyMode::Chunked;
            self.state = HttpDecodeState::Body;
        } else if let Some(length) = content_length {
            if length > MAX_FETCH_TOTAL_BYTES {
                return self.fail(FetchError::ResponseTooLarge);
            }
            self.body_mode = HttpBodyMode::ContentLength { remaining: length };
            self.state = if length == 0 {
                HttpDecodeState::Complete
            } else {
                HttpDecodeState::Body
            };
        } else {
            return self.fail(FetchError::Protocol);
        }
        Ok(())
    }

    fn decode_chunked(
        &mut self,
        input: &[u8],
        output: &mut [u8],
    ) -> Result<(usize, usize), FetchError> {
        let mut consumed = 0usize;
        let mut written = 0usize;
        while consumed < input.len() && self.state == HttpDecodeState::Body {
            match self.chunk_state {
                ChunkState::Size => {
                    let byte = input[consumed];
                    consumed += 1;
                    if byte == b'\r' {
                        if self.chunk_size_len == 0 {
                            return self.fail(FetchError::Protocol);
                        }
                        let size = parse_hex_u32(&self.chunk_size[..self.chunk_size_len])
                            .ok_or(FetchError::Protocol)?;
                        if size > MAX_FETCH_TOTAL_BYTES.saturating_sub(self.body_bytes) {
                            return self.fail(FetchError::ResponseTooLarge);
                        }
                        self.chunk_size.fill(0);
                        self.chunk_size_len = 0;
                        self.chunk_state = ChunkState::SizeLf { size };
                    } else {
                        if self.chunk_size_len == self.chunk_size.len() || !byte.is_ascii_hexdigit()
                        {
                            return self.fail(FetchError::Protocol);
                        }
                        self.chunk_size[self.chunk_size_len] = byte;
                        self.chunk_size_len += 1;
                    }
                }
                ChunkState::SizeLf { size } => {
                    if input[consumed] != b'\n' {
                        return self.fail(FetchError::Protocol);
                    }
                    consumed += 1;
                    self.chunk_state = if size == 0 {
                        ChunkState::FinalCr
                    } else {
                        ChunkState::Data { remaining: size }
                    };
                }
                ChunkState::Data { remaining } => {
                    if written == output.len() {
                        break;
                    }
                    let count = (remaining as usize)
                        .min(input.len() - consumed)
                        .min(output.len() - written);
                    output[written..written + count]
                        .copy_from_slice(&input[consumed..consumed + count]);
                    consumed += count;
                    written += count;
                    self.body_bytes = self.body_bytes.saturating_add(count as u32);
                    self.chunk_state = if count as u32 == remaining {
                        ChunkState::DataCr
                    } else {
                        ChunkState::Data {
                            remaining: remaining - count as u32,
                        }
                    };
                }
                ChunkState::DataCr => {
                    if input[consumed] != b'\r' {
                        return self.fail(FetchError::Protocol);
                    }
                    consumed += 1;
                    self.chunk_state = ChunkState::DataLf;
                }
                ChunkState::DataLf => {
                    if input[consumed] != b'\n' {
                        return self.fail(FetchError::Protocol);
                    }
                    consumed += 1;
                    self.chunk_state = ChunkState::Size;
                }
                ChunkState::FinalCr => {
                    if input[consumed] != b'\r' {
                        return self.fail(FetchError::Protocol);
                    }
                    consumed += 1;
                    self.chunk_state = ChunkState::FinalEndLf;
                }
                ChunkState::FinalEndLf => {
                    if input[consumed] != b'\n' {
                        return self.fail(FetchError::Protocol);
                    }
                    consumed += 1;
                    self.state = HttpDecodeState::Complete;
                }
            }
        }
        Ok((consumed, written))
    }

    fn fail<T>(&mut self, error: FetchError) -> Result<T, FetchError> {
        self.headers.fill(0);
        self.chunk_size.fill(0);
        self.header_len = 0;
        self.chunk_size_len = 0;
        self.state = HttpDecodeState::Failed(error);
        Err(error)
    }
}

impl Default for HttpResponseDecoder {
    fn default() -> Self {
        Self::new()
    }
}

fn next_crlf_line<'a>(bytes: &'a [u8], cursor: &mut usize) -> Option<&'a [u8]> {
    let start = *cursor;
    let relative = bytes[start..]
        .windows(2)
        .position(|window| window == b"\r\n")?;
    let end = start + relative;
    *cursor = end + 2;
    Some(&bytes[start..end])
}

fn parse_three_digits(bytes: &[u8]) -> Option<u16> {
    if bytes.len() != 3 || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some(
        u16::from(bytes[0] - b'0') * 100
            + u16::from(bytes[1] - b'0') * 10
            + u16::from(bytes[2] - b'0'),
    )
}

fn parse_decimal_u32(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() || !bytes.iter().all(u8::is_ascii_digit) {
        return None;
    }
    bytes.iter().try_fold(0u32, |value, byte| {
        value.checked_mul(10)?.checked_add(u32::from(*byte - b'0'))
    })
}

fn parse_hex_u32(bytes: &[u8]) -> Option<u32> {
    bytes.iter().try_fold(0u32, |value, byte| {
        let digit = match byte {
            b'0'..=b'9' => u32::from(*byte - b'0'),
            b'a'..=b'f' => u32::from(*byte - b'a') + 10,
            b'A'..=b'F' => u32::from(*byte - b'A') + 10,
            _ => return None,
        };
        value.checked_mul(16)?.checked_add(digit)
    })
}

fn trim_ascii_space(mut bytes: &[u8]) -> &[u8] {
    while bytes
        .first()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
    {
        bytes = &bytes[1..];
    }
    while bytes
        .last()
        .is_some_and(|byte| *byte == b' ' || *byte == b'\t')
    {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn ascii_eq_ignore_case(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn is_header_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(byte, b'!' | b'#'..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^'..=b'`' | b'|' | b'~')
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeBackend {
        available: bool,
        state: BackendPoll,
        body: &'static [u8],
        offset: usize,
        cancellations: u8,
        last_pins: FetchPinSet,
    }

    impl FetchBackend for FakeBackend {
        fn available(&self) -> bool {
            self.available
        }

        fn start(
            &mut self,
            _: FetchRequestId,
            _: &str,
            pins: FetchPinSet,
        ) -> Result<(), FetchError> {
            self.offset = 0;
            self.last_pins = pins;
            Ok(())
        }

        fn poll(&mut self, _: FetchRequestId) -> BackendPoll {
            self.state
        }

        fn read(&mut self, _: FetchRequestId, dst: &mut [u8]) -> Result<usize, FetchError> {
            let count = dst.len().min(self.body.len().saturating_sub(self.offset));
            dst[..count].copy_from_slice(&self.body[self.offset..self.offset + count]);
            self.offset += count;
            Ok(count)
        }

        fn cancel(&mut self, _: FetchRequestId) {
            self.cancellations += 1;
        }
    }

    fn service() -> AppFetchService<FakeBackend> {
        AppFetchService::new(FakeBackend {
            available: true,
            state: BackendPoll::Pending,
            body: b"weather",
            offset: 0,
            cancellations: 0,
            last_pins: FetchPinSet::empty(),
        })
    }

    fn permissions() -> FetchAllowlist {
        let mut list = FetchAllowlist::empty();
        list.push(FetchOrigin::parse("https://api.example.com").unwrap())
            .unwrap();
        list
    }

    #[test]
    fn origins_are_canonical_and_bounded() {
        let origin = FetchOrigin::parse("https://api.example.com:8443").unwrap();
        assert_eq!(origin.hostname(), "api.example.com");
        assert_eq!(origin.port(), 8443);
        assert_eq!(origin.scheme(), FetchScheme::Https);
        assert_eq!(
            FetchOrigin::parse("HTTPS://api.example.com"),
            Err(OriginError::UnsupportedScheme)
        );
        assert_eq!(
            FetchOrigin::parse("https://*.example.com"),
            Err(OriginError::Wildcard)
        );
        assert_eq!(
            FetchOrigin::parse("https://user@example.com"),
            Err(OriginError::UserInfo)
        );
        assert_eq!(
            FetchOrigin::parse("https://api.example.com:443"),
            Err(OriginError::NonCanonical)
        );
    }

    #[test]
    fn fetch_url_is_split_once_for_policy_and_transport() {
        let target = parse_fetch_url("https://api.example.com:8443/v1/data?q=1").unwrap();
        assert_eq!(target.scheme(), FetchScheme::Https);
        assert_eq!(target.hostname(), "api.example.com");
        assert_eq!(target.port(), 8443);
        assert_eq!(target.request_target_suffix(), "/v1/data?q=1");

        let root = parse_fetch_url("https://api.example.com").unwrap();
        assert_eq!(root.request_target_suffix(), "");
        let query = parse_fetch_url("https://api.example.com?units=metric").unwrap();
        assert_eq!(query.request_target_suffix(), "?units=metric");
    }

    #[test]
    fn fetch_url_rejects_non_wire_safe_or_fragmented_inputs() {
        for url in [
            "https://api.example.com/a b",
            "https://api.example.com/天気",
            "https://api.example.com/data#local",
            "https:///missing-host",
        ] {
            assert_eq!(parse_fetch_url(url), Err(OriginError::Malformed));
        }
    }

    #[test]
    fn fetch_get_encoder_handles_root_query_and_explicit_port() {
        let mut output = [0u8; 768];
        let len = encode_fetch_get_request("https://api.example.com", &mut output).unwrap();
        assert_eq!(
            &output[..len],
            b"GET / HTTP/1.1\r\nHost: api.example.com\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        );

        let len =
            encode_fetch_get_request("https://api.example.com:8443?units=metric", &mut output)
                .unwrap();
        assert_eq!(
            &output[..len],
            b"GET /?units=metric HTTP/1.1\r\nHost: api.example.com:8443\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn fetch_get_encoder_sizes_before_touching_destination() {
        let mut output = [0xa5; 8];
        assert_eq!(
            encode_fetch_get_request("https://api.example.com/path", &mut output),
            Err(FetchError::BufferTooLarge)
        );
        assert_eq!(output, [0xa5; 8]);
    }

    #[test]
    fn fetch_get_encoder_injects_bearer_token() {
        use crate::vault::CredentialInjection;
        let mut output = [0u8; 768];
        let injection = CredentialInjection::BearerToken {
            token: b"tok_live_abc123",
        };
        let len = encode_fetch_get_request_with_injection(
            "https://api.example.com/v1/data",
            &injection,
            &mut output,
        )
        .unwrap();
        assert_eq!(
            &output[..len],
            b"GET /v1/data HTTP/1.1\r\nHost: api.example.com\r\nAuthorization: Bearer tok_live_abc123\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn fetch_get_encoder_injects_api_key_header() {
        use crate::vault::CredentialInjection;
        let mut output = [0u8; 768];
        let injection = CredentialInjection::ApiKeyHeader {
            name: b"X-Api-Key",
            value: b"secretkey123",
        };
        let len = encode_fetch_get_request_with_injection(
            "https://api.example.com",
            &injection,
            &mut output,
        )
        .unwrap();
        assert_eq!(
            &output[..len],
            b"GET / HTTP/1.1\r\nHost: api.example.com\r\nX-Api-Key: secretkey123\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        );
    }

    #[test]
    fn fetch_get_encoder_refuses_mqtt_login() {
        use crate::vault::CredentialInjection;
        let mut output = [0u8; 768];
        let injection = CredentialInjection::MqttLogin {
            username: b"device01",
            password: b"pw",
        };
        assert_eq!(
            encode_fetch_get_request_with_injection(
                "https://api.example.com",
                &injection,
                &mut output
            ),
            Err(FetchError::Denied)
        );
    }

    #[test]
    fn fetch_get_encoder_sizes_credential_before_touching_destination() {
        use crate::vault::CredentialInjection;
        // Room for the plain request but not the added Authorization header.
        let mut output = [0xa5; 90];
        let injection = CredentialInjection::BearerToken {
            token: b"tok_live_abc123456789",
        };
        assert_eq!(
            encode_fetch_get_request_with_injection(
                "https://api.example.com",
                &injection,
                &mut output
            ),
            Err(FetchError::BufferTooLarge)
        );
        assert_eq!(output, [0xa5; 90]);
    }

    #[test]
    fn allowlist_rejects_duplicates_and_overflow() {
        let mut list = FetchAllowlist::empty();
        let one = FetchOrigin::parse("https://one.example").unwrap();
        list.push(one).unwrap();
        assert_eq!(list.push(one), Err(AllowlistError::Duplicate));
        for name in ["two", "three", "four"] {
            let mut text = std::string::String::from("https://");
            text.push_str(name);
            text.push_str(".example");
            list.push(FetchOrigin::parse(&text).unwrap()).unwrap();
        }
        assert_eq!(
            list.push(FetchOrigin::parse("https://five.example").unwrap()),
            Err(AllowlistError::TooMany)
        );
    }

    #[test]
    fn manifest_fetch_permission_is_root_scoped_versioned_and_bounded() {
        let v2 = parse_manifest_fetch_permission(
            r#"{
                "decoy":{"permissions":{"network":{"origins":["https://evil.example"]}}},
                "permissions":{"fs":"sandbox","network":{"origins":[
                    "https://weather.example", "https://api.example:8443"
                ]}},
                "assets":[]
            }"#,
            2,
        )
        .unwrap();
        assert_eq!(v2.legacy, None);
        assert_eq!(v2.allowlist.len(), 2);
        assert!(v2
            .allowlist
            .contains(FetchOrigin::parse("https://weather.example").unwrap()));

        let v1 = parse_manifest_fetch_permission(
            r#"{"permissions":{"network":false},"value":1.25e+2}"#,
            1,
        )
        .unwrap();
        assert_eq!(v1.legacy, Some(false));
        assert!(v1.allowlist.is_empty());

        let absent = parse_manifest_fetch_permission(
            r#"{"nested":{"network":{"origins":["https://evil.example"]}}}"#,
            2,
        )
        .unwrap();
        assert!(absent.allowlist.is_empty());
    }

    #[test]
    fn manifest_fetch_permission_rejects_ambiguous_or_malformed_shapes() {
        for manifest in [
            r#"{"permissions":{"network":{"origins":["https://one.example"],"extra":true}}}"#,
            r#"{"permissions":{"network":{"origins":["https://*.example"]}}}"#,
            r#"{"permissions":{"network":{"origins":["https:\/\/api.example"]}}}"#,
            r#"{"permissions":{"network":{"origins":["https://one.example","https://one.example"]}}}"#,
            r#"{"permissions":{"network":{"origins":[]},"network":{"origins":[]}}}"#,
            r#"{"permissions":{},"permissions":{}}"#,
            r#"{"permissions":{"network":{"origins":[]}},}"#,
        ] {
            assert!(parse_manifest_fetch_permission(manifest, 2).is_err());
        }
        assert!(parse_manifest_fetch_permission(
            r#"{"permissions":{"network":{"origins":[]}}}"#,
            1
        )
        .is_err());
        assert!(
            parse_manifest_fetch_permission(r#"{"permissions":{"network":false}}"#, 2).is_err()
        );
    }

    #[test]
    fn manifest_spki_pins_are_fixed_canonical_and_rotation_bounded() {
        const CURRENT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        const NEXT: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let manifest = std::format!(
            r#"{{"permissions":{{"network":{{"origins":[{{"origin":"https://secure.example","spki_sha256":["{CURRENT}","{NEXT}"]}},"https://sim.example"]}}}}}}"#
        );
        let parsed = parse_manifest_fetch_permission(&manifest, 2).unwrap();
        assert_eq!(parsed.allowlist.len(), 2);
        let pins = parsed.pins.get(0).unwrap();
        assert_eq!(pins.len(), 2);
        assert!(pins.contains(SpkiSha256::parse_hex(CURRENT).unwrap()));
        assert!(!parsed.pins.complete_for(&parsed.allowlist));

        for bad in [
            std::format!(
                r#"{{"permissions":{{"network":{{"origins":[{{"origin":"http://secure.example","spki_sha256":["{CURRENT}"]}}]}}}}}}"#
            ),
            r#"{"permissions":{"network":{"origins":[{"origin":"https://secure.example","spki_sha256":[]}]}}}"#
                .to_string(),
            std::format!(
                r#"{{"permissions":{{"network":{{"origins":[{{"origin":"https://secure.example","spki_sha256":["{CURRENT}","{CURRENT}"]}}]}}}}}}"#
            ),
            std::format!(
                r#"{{"permissions":{{"network":{{"origins":[{{"origin":"https://secure.example","spki_sha256":["{CURRENT}","{NEXT}","cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"]}}]}}}}}}"#
            ),
        ] {
            assert!(parse_manifest_fetch_permission(&bad, 2).is_err());
        }
        assert_eq!(
            SpkiSha256::parse_hex(
                "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
            ),
            Err(ManifestFetchError::InvalidPin)
        );
    }

    #[test]
    fn extracts_exact_spki_der_and_compares_every_pin_slot() {
        const CERTIFICATE: &[u8] = &[
            0x30, 0x23, // Certificate
            0x30, 0x1b, // TBSCertificate
            0xa0, 0x03, 0x02, 0x01, 0x02, // version v3
            0x02, 0x01, 0x01, // serial
            0x30, 0x00, // signature algorithm
            0x30, 0x00, // issuer
            0x30, 0x00, // validity
            0x30, 0x00, // subject
            0x30, 0x09, // SubjectPublicKeyInfo
            0x30, 0x03, 0x06, 0x01, 0x2a, // algorithm identifier
            0x03, 0x02, 0x00, 0xaa, // subjectPublicKey
            0x30, 0x00, // certificate signature algorithm
            0x03, 0x02, 0x00, 0xaa, // certificate signature
        ];
        assert_eq!(
            extract_certificate_spki_der(CERTIFICATE),
            Ok(&CERTIFICATE[20..31])
        );

        let digest = [0x5a; 32];
        let mut set = FetchPinSet::empty();
        set.push(SpkiSha256(digest)).unwrap();
        set.push(SpkiSha256([0xa5; 32])).unwrap();
        assert!(set.matches_digest(&digest));
        assert!(!set.matches_digest(&[0; 32]));

        let mut indefinite = CERTIFICATE.to_vec();
        indefinite[1] = 0x80;
        assert_eq!(
            extract_certificate_spki_der(&indefinite),
            Err(SpkiDerError::Malformed)
        );
        assert_eq!(
            extract_certificate_spki_der(&vec![0; MAX_FETCH_CERTIFICATE_BYTES + 1]),
            Err(SpkiDerError::CertificateTooLarge)
        );
    }

    #[test]
    fn extracts_only_uncompressed_p256_spki_keys() {
        let mut spki = [0u8; 91];
        spki[..26].copy_from_slice(&[
            0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06,
            0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00,
        ]);
        spki[26] = 0x04;
        for (index, byte) in spki[27..].iter_mut().enumerate() {
            *byte = index as u8;
        }
        let key = extract_p256_public_key_from_spki_der(&spki).unwrap();
        assert_eq!(key[0], 0x04);
        assert_eq!(key[64], 63);

        let mut wrong_curve = spki;
        wrong_curve[22] ^= 1;
        assert_eq!(
            extract_p256_public_key_from_spki_der(&wrong_curve),
            Err(SpkiDerError::UnsupportedKey)
        );
        let mut compressed = spki;
        compressed[26] = 0x02;
        assert_eq!(
            extract_p256_public_key_from_spki_der(&compressed),
            Err(SpkiDerError::UnsupportedKey)
        );
    }

    #[cfg(feature = "app_fetch_tls_verifier")]
    #[test]
    fn verifies_tls13_p256_certificate_signature_known_answer() {
        use p256::ecdsa::{signature::Signer, Signature, SigningKey};

        let signing_key = SigningKey::from_bytes((&[7u8; 32]).into()).unwrap();
        let encoded_key = signing_key.verifying_key().to_encoded_point(false);
        let public_key: &[u8; 65] = encoded_key.as_bytes().try_into().unwrap();
        let transcript_hash = [0x5a; 32];
        let mut message = [0u8; 130];
        message[..64].fill(0x20);
        message[64..98].copy_from_slice(b"TLS 1.3, server CertificateVerify\x00");
        message[98..].copy_from_slice(&transcript_hash);
        let signature: Signature = signing_key.sign(&message);
        let signature_der = signature.to_der();

        assert!(verify_p256_tls13_certificate_signature(
            public_key,
            &transcript_hash,
            signature_der.as_bytes()
        ));
        let mut wrong_hash = transcript_hash;
        wrong_hash[0] ^= 1;
        assert!(!verify_p256_tls13_certificate_signature(
            public_key,
            &wrong_hash,
            signature_der.as_bytes()
        ));
        assert!(!verify_p256_tls13_certificate_signature(
            public_key,
            &transcript_hash,
            &[0; 8]
        ));
    }

    #[test]
    fn start_is_default_denied_and_one_per_app() {
        let mut service = service();
        let app = AppContext {
            app_id: 7,
            generation: 2,
        };
        assert_eq!(
            service.start(
                app,
                &FetchAllowlist::empty(),
                "https://api.example.com/v1",
                0
            ),
            Err(FetchError::Denied)
        );
        let id = service
            .start(app, &permissions(), "https://api.example.com/v1?q=tokyo", 0)
            .unwrap();
        assert_eq!(
            service.start(app, &permissions(), "https://api.example.com/v2", 1),
            Err(FetchError::Busy)
        );
        assert_eq!(service.poll(app, id, 1), Ok(FetchPoll::Pending));
    }

    #[test]
    fn external_controller_passes_only_the_selected_origin_pins() {
        let app = AppContext {
            app_id: 9,
            generation: 3,
        };
        let mut backend = FakeBackend {
            available: true,
            state: BackendPoll::Pending,
            body: b"",
            offset: 0,
            cancellations: 0,
            last_pins: FetchPinSet::empty(),
        };
        let mut pin_set = FetchPinSet::empty();
        let pin = SpkiSha256::from_bytes([0x5a; 32]);
        pin_set.push(pin).unwrap();
        let mut pins = FetchPinTable::empty();
        pins.sets[0] = pin_set;
        let mut controller = AppFetchController::new();
        controller
            .start_pinned(
                &mut backend,
                app,
                &permissions(),
                &pins,
                "https://api.example.com/v1",
                0,
            )
            .unwrap();
        assert_eq!(backend.last_pins, pin_set);
        assert!(backend.last_pins.contains(pin));
    }

    #[test]
    fn copied_reads_cancel_timeout_and_reject_stale_or_foreign_ids() {
        let mut service = service();
        let app = AppContext {
            app_id: 7,
            generation: 2,
        };
        let id = service
            .start(app, &permissions(), "https://api.example.com/v1", 10)
            .unwrap();
        let mut dst = [0; 4];
        assert_eq!(service.read(app, id, &mut dst), Ok(4));
        assert_eq!(&dst, b"weat");
        assert_eq!(service.diagnostics(app, id, 12).unwrap().bytes_read, 4);
        assert_eq!(
            service.poll(
                AppContext {
                    app_id: 8,
                    generation: 2
                },
                id,
                12
            ),
            Err(FetchError::ForeignRequest)
        );
        assert_eq!(
            service.poll(app, id, 10 + u64::from(MAX_FETCH_DURATION_MS) + 1),
            Ok(FetchPoll::Failed(FetchError::Timeout))
        );
        service.teardown();
        assert_eq!(service.poll(app, id, 20), Err(FetchError::StaleRequest));
        assert_eq!(service.backend_mut().cancellations, 2);
    }

    #[test]
    fn unavailable_backend_is_stable() {
        let mut service = service();
        service.backend_mut().available = false;
        assert_eq!(
            service.start(
                AppContext {
                    app_id: 1,
                    generation: 1
                },
                &permissions(),
                "https://api.example.com/",
                0
            ),
            Err(FetchError::Unavailable)
        );
    }

    #[test]
    fn teardown_cancels_every_app_and_invalidates_the_generation() {
        let mut service = service();
        let first_app = AppContext {
            app_id: 7,
            generation: 1,
        };
        let second_app = AppContext {
            app_id: 8,
            generation: 1,
        };
        let first = service
            .start(first_app, &permissions(), "https://api.example.com/one", 10)
            .unwrap();
        let second = service
            .start(
                second_app,
                &permissions(),
                "https://api.example.com/two",
                11,
            )
            .unwrap();

        service.teardown();
        assert_eq!(service.backend_mut().cancellations, 2);
        assert_eq!(
            service.poll(first_app, first, 12),
            Err(FetchError::StaleRequest)
        );
        assert_eq!(
            service.poll(second_app, second, 12),
            Err(FetchError::StaleRequest)
        );

        let restarted = service
            .start(
                first_app,
                &permissions(),
                "https://api.example.com/restarted",
                13,
            )
            .unwrap();
        assert_ne!(restarted.raw() >> 16, first.raw() >> 16);
    }

    #[test]
    fn reconstructed_service_uses_persisted_generation() {
        let app = AppContext {
            app_id: 7,
            generation: 41,
        };
        let mut service = service();
        service.reset_generation(41);
        let first = service
            .start(app, &permissions(), "https://api.example.com/one", 0)
            .unwrap();
        assert_eq!(first.raw() >> 16, 41);

        service.reset_generation(42);
        assert_eq!(service.poll(app, first, 1), Err(FetchError::StaleRequest));
        let second_app = AppContext {
            generation: 42,
            ..app
        };
        let second = service
            .start(second_app, &permissions(), "https://api.example.com/two", 1)
            .unwrap();
        assert_eq!(second.raw() >> 16, 42);
    }

    #[test]
    fn transport_mailbox_streams_one_chunk_and_zeroizes_on_release() {
        let request = FetchRequestId::from_raw(0x002a_0001);
        let mut mailbox = FetchTransportMailbox::new();
        mailbox
            .submit(
                request,
                "https://api.example.com/weather?q=tokyo",
                FetchPinSet::empty(),
            )
            .unwrap();
        assert_eq!(mailbox.state(), FetchTransportState::Queued);
        let mut url = [0u8; MAX_FETCH_URL_BYTES];
        let command = mailbox.take_command(&mut url).unwrap().unwrap();
        assert_eq!(command.request, request);
        assert_eq!(
            &url[..usize::from(command.url_len)],
            b"https://api.example.com/weather?q=tokyo"
        );
        mailbox.publish_headers(request, 200).unwrap();
        assert_eq!(mailbox.poll(request), BackendPoll::Headers { status: 200 });
        mailbox.publish_body(request, b"weather").unwrap();
        let mut first = [0u8; 3];
        assert_eq!(mailbox.read_body(request, &mut first), Ok(3));
        assert_eq!(&first, b"wea");
        let mut rest = [0u8; 8];
        assert_eq!(mailbox.read_body(request, &mut rest), Ok(4));
        assert_eq!(&rest[..4], b"ther");
        assert_eq!(mailbox.state(), FetchTransportState::Running);
        mailbox.complete(request).unwrap();
        assert_eq!(mailbox.poll(request), BackendPoll::Complete);
        mailbox.release_terminal(request).unwrap();
        assert_eq!(mailbox.state(), FetchTransportState::Idle);
    }

    #[test]
    fn transport_mailbox_records_headers_observation_for_the_producer() {
        let request = FetchRequestId::from_raw(0x002a_0007);
        let mut mailbox = FetchTransportMailbox::new();
        mailbox
            .submit(
                request,
                "https://api.example.com/weather",
                FetchPinSet::empty(),
            )
            .unwrap();
        let mut url = [0u8; MAX_FETCH_URL_BYTES];
        mailbox.take_command(&mut url).unwrap().unwrap();
        mailbox.publish_headers(request, 200).unwrap();
        // The read-only poll never records observation; only the VM-facing
        // `poll_mut` does, and stale requests do not count.
        assert!(!mailbox.headers_polled());
        assert_eq!(mailbox.poll(request), BackendPoll::Headers { status: 200 });
        assert!(!mailbox.headers_polled());
        assert_eq!(
            mailbox.poll_mut(FetchRequestId::from_raw(0x002a_0008)),
            BackendPoll::Failed(FetchError::StaleRequest)
        );
        assert!(!mailbox.headers_polled());
        assert_eq!(
            mailbox.poll_mut(request),
            BackendPoll::Headers { status: 200 }
        );
        assert!(mailbox.headers_polled());
        // The flag clears with the next request's payload reset.
        mailbox.publish_body(request, b"ok").unwrap();
        let mut drained = [0u8; 2];
        assert_eq!(mailbox.read_body(request, &mut drained), Ok(2));
        mailbox.complete(request).unwrap();
        mailbox.release_terminal(request).unwrap();
        let next = FetchRequestId::from_raw(0x002a_0009);
        mailbox
            .submit(next, "https://api.example.com/next", FetchPinSet::empty())
            .unwrap();
        assert!(!mailbox.headers_polled());
    }

    #[test]
    fn transport_mailbox_requires_executor_cancel_ack() {
        let request = FetchRequestId::from_raw(0x002a_0002);
        let mut mailbox = FetchTransportMailbox::new();
        mailbox
            .submit(
                request,
                "https://api.example.com/slow",
                FetchPinSet::empty(),
            )
            .unwrap();
        assert_eq!(
            mailbox.submit(
                FetchRequestId::from_raw(0x002a_0003),
                "https://api.example.com/other",
                FetchPinSet::empty(),
            ),
            Err(FetchError::Busy)
        );
        mailbox.request_cancel(request).unwrap();
        assert!(mailbox.cancel_requested(request));
        assert_eq!(mailbox.state(), FetchTransportState::CancelRequested);
        mailbox.acknowledge_cancel(request).unwrap();
        assert_eq!(mailbox.state(), FetchTransportState::Idle);
        assert_eq!(
            mailbox.poll(request),
            BackendPoll::Failed(FetchError::StaleRequest)
        );
    }

    #[test]
    fn release_destination_filter_rejects_local_special_and_rebind_answers() {
        for address in [
            [0, 0, 0, 0],
            [10, 1, 2, 3],
            [127, 0, 0, 1],
            [169, 254, 1, 1],
            [172, 16, 0, 1],
            [192, 168, 0, 1],
            [224, 0, 0, 1],
        ] {
            assert!(!release_ipv4_allowed(address));
        }
        assert!(release_ipv4_allowed([203, 0, 113, 10]));
        assert!(!release_ipv6_allowed([0; 16]));
        let mut loopback = [0; 16];
        loopback[15] = 1;
        assert!(!release_ipv6_allowed(loopback));
        assert!(!release_ipv6_allowed([
            0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1
        ]));
        assert!(release_ipv6_allowed([
            0x20, 1, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1
        ]));
    }

    #[test]
    fn http_content_length_decodes_partial_headers_and_bounded_output() {
        let response = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nX-Test: ok\r\n\r\nhello";
        let mut decoder = HttpResponseDecoder::new();
        let first = decoder.push(&response[..17], &mut []).unwrap();
        assert_eq!(first.state, HttpDecodeState::Headers);
        assert_eq!(first.consumed, 17);

        let mut offset = 17;
        let mut body = [0u8; 5];
        let mut body_len = 0;
        while decoder.state() != HttpDecodeState::Complete {
            let end = (offset + 7).min(response.len());
            let output_end = (body_len + 2).min(body.len());
            let progress = decoder
                .push(&response[offset..end], &mut body[body_len..output_end])
                .unwrap();
            assert!(progress.consumed != 0 || progress.written != 0);
            offset += progress.consumed;
            body_len += progress.written;
        }
        assert_eq!(&body, b"hello");
        assert_eq!(decoder.status(), Some(200));
        assert_eq!(decoder.body_bytes(), 5);
    }

    #[test]
    fn http_chunked_decodes_across_every_boundary_without_leaking_metadata() {
        let response = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        let mut decoder = HttpResponseDecoder::new();
        let mut offset = 0usize;
        let mut body = [0u8; 5];
        let mut body_len = 0usize;
        while decoder.state() != HttpDecodeState::Complete {
            let end = (offset + 1).min(response.len());
            let output_end = (body_len + 1).min(body.len());
            let progress = decoder
                .push(&response[offset..end], &mut body[body_len..output_end])
                .unwrap();
            offset += progress.consumed;
            body_len += progress.written;
            assert!(offset <= response.len());
        }
        assert_eq!(&body, b"abcde");
        assert_eq!(offset, response.len());
    }

    #[test]
    fn http_framing_conflicts_extensions_and_oversize_fail_closed() {
        for response in [
            &b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nx"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n"[..],
            &b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\n\r\n"[..],
        ] {
            let mut decoder = HttpResponseDecoder::new();
            assert_eq!(
                decoder.push(response, &mut [0; 8]),
                Err(FetchError::Protocol)
            );
            assert_eq!(
                decoder.state(),
                HttpDecodeState::Failed(FetchError::Protocol)
            );
            assert_eq!(
                decoder.push(b"ignored", &mut [0; 8]),
                Err(FetchError::Protocol)
            );
        }

        let mut chunked = HttpResponseDecoder::new();
        assert_eq!(
            chunked.push(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1;x\r\na\r\n0\r\n\r\n",
                &mut [0; 8]
            ),
            Err(FetchError::Protocol)
        );

        let mut oversized = HttpResponseDecoder::new();
        assert_eq!(
            oversized.push(&[b'a'; MAX_FETCH_HEADER_BYTES + 1], &mut []),
            Err(FetchError::ResponseTooLarge)
        );
        oversized.reset();
        assert_eq!(oversized.state(), HttpDecodeState::Headers);
    }
}
