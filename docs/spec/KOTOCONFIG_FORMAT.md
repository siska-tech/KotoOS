# KotoConfig Public Settings Format

- Status: format 1.0 implemented by KOTO-0223 increment 1
- Byte order: little-endian
- Maximum encoded bytes: 280

The portable ConfigService owns this format. Platform adapters store complete
snapshots and never expose their backing paths to applications. Secret settings
such as future Wi-Fi credentials do not use this format.

## Header (`KCF1`, 24 bytes)

| Offset | Size | Field | Rule |
| --: | --: | :-- | :-- |
| 0 | 4 | magic | ASCII `KCF1` |
| 4 | 2 | format major | 1 |
| 6 | 2 | format minor | 0 |
| 8 | 2 | total length | Exact file/record length |
| 10 | 1 | setting count | 1..8 |
| 11 | 1 | record stride | 32 |
| 12 | 4 | config generation | Nonzero, wrapping maximum to 1 |
| 16 | 4 | checksum | FNV-1a over the complete packet with this field treated as zero |
| 20 | 4 | locale generation | Nonzero; changes only when locale changes |

Format 1.0 rejects unsupported versions, truncation/trailing bytes, zero
generations, excessive counts, bad stride, duplicate/zero keys, invalid record
padding, invalid UTF-8, unsupported locale, and checksum mismatch. The caller
loads `ConfigService::default()` (`en-US`, generations 1) after rejection.

## Public setting record (32 bytes)

| Offset | Size | Field | Rule |
| --: | --: | :-- | :-- |
| 0 | 2 | key | Stable nonzero key |
| 2 | 1 | kind | 1 is UTF-8; other values are opaque compatible kinds |
| 3 | 1 | value length | 0..24 |
| 4 | 4 | reserved | zero |
| 8 | 24 | value | Used bytes followed by zero padding |

Key 1 is `system.locale`, kind UTF-8, and accepts `en-US`, `ja-JP`, plus the
simulator/test-only `qps-ploc`. Unknown keys are preserved byte-for-byte across a
compatible decode/re-encode but are not applied without a registered validator.

Wi-Fi SSIDs, security modes, credentials, profile metadata, and connection
history are not public-setting records. The bounded secret-provider format is
owned by a separate implementation issue and must follow the at-rest and
zeroization rules in the
[KotoConfig Wi-Fi extension contract](../architecture/KOTOCONFIG_WIFI_EXTENSION.md).

## Storage commit policy

The simulator adapter stores `config-a.bin` and `config-b.bin` beneath the
OS-owned `dev.koto.config` namespace. It loads the valid slot with the newest
wrapping generation and writes the next snapshot to the older/invalid slot.
Thus a torn write invalidates at most the replacement slot. The Pico adapter
must provide the equivalent two-slot behavior using an 8.3-safe filename and
bounded reads. KOTO-0223 implements this as root-level `KCFGA.BIN` and
`KCFGB.BIN`; physical power-loss validation remains before issue completion.
