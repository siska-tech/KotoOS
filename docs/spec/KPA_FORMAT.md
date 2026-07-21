# KPA Package Format

This document defines KPA v1, the first concrete binary `.kpa` archive format for KotoOS packages.

The current `*.kpa.json` files are source manifests used by KotoSim and future packer tools. They are not the final archive format. A packer reads one manifest plus referenced asset files and emits one `.kpa` file.

## Design Goals

- Keep app distribution as one file, satisfying FR-PKG-1.
- Store bytecode and assets in deterministic sequential ranges, satisfying FR-PKG-2.
- Allow a host packer to build archives from Rust/script build outputs and manifest metadata, satisfying FR-PKG-3.
- Make the loader usable on RP2040-class hardware with small SRAM buffers and slow random SD reads.

## File Layout

KPA v1 is little-endian. All offsets are absolute byte offsets from the start of the `.kpa` file.

```text
+--------------------------+ offset 0
| KpaHeader                |
+--------------------------+ header.table_offset
| KpaEntry[entry_count]    |
+--------------------------+ header.string_table_offset
| UTF-8 string table       |
+--------------------------+ header.metadata_offset
| UTF-8 manifest JSON copy |
+--------------------------+ first asset offset, 4096-byte aligned
| Asset payload 0          |
| padding                  |
| Asset payload 1          |
| padding                  |
| ...                      |
+--------------------------+ end of file
```

The header, entry table, string table, and metadata copy form the metadata region. The metadata region may be read with small random accesses during package discovery. Asset payloads are optimized for sequential reads.

## Header

`KpaHeader` is exactly 64 bytes.

| Offset | Size | Field               | Description                                             |
| :----- | :--- | :------------------ | :------------------------------------------------------ |
| 0      | 4    | magic               | ASCII `KPA1`.                                           |
| 4      | 2    | version_major       | `1` for this document.                                  |
| 6      | 2    | version_minor       | `0` for this document.                                  |
| 8      | 4    | header_size         | Must be `64`.                                           |
| 12     | 4    | flags               | Reserved. Must be `0` in v1.                            |
| 16     | 4    | entry_count         | Number of `KpaEntry` records.                           |
| 20     | 4    | table_offset        | Offset of the entry table. Must be 64 in v1.            |
| 24     | 4    | string_table_offset | Offset of the UTF-8 string table.                       |
| 28     | 4    | string_table_size   | Size of the string table in bytes.                      |
| 32     | 4    | metadata_offset     | Offset of the manifest JSON copy.                       |
| 36     | 4    | metadata_size       | Size of the manifest JSON copy in bytes.                |
| 40     | 4    | first_asset_offset  | Offset of the first payload. Must be 4096-byte aligned. |
| 44     | 4    | package_size        | Total file size in bytes.                               |
| 48     | 16   | reserved            | Must be zero.                                           |

Loaders must reject archives with an unsupported version, bad magic, nonzero reserved bytes, non-monotonic offsets, or offsets beyond `package_size`.

## Entry Table

Each `KpaEntry` is exactly 64 bytes.

| Offset | Size | Field        | Description                                                  |
| :----- | :--- | :----------- | :----------------------------------------------------------- |
| 0      | 4    | path_offset  | Offset into the string table.                                |
| 4      | 4    | path_len     | UTF-8 path length in bytes, without trailing NUL.            |
| 8      | 4    | type         | Asset type ID.                                               |
| 12     | 4    | flags        | Entry flags.                                                 |
| 16     | 4    | data_offset  | Absolute payload offset.                                     |
| 20     | 4    | data_size    | Payload size in bytes.                                       |
| 24     | 4    | alignment    | Payload alignment used by the packer.                        |
| 28     | 4    | reserved0    | Must be zero.                                                |
| 32     | 32   | content_hash | Reserved for SHA-256 or zeroes until hashing is implemented. |

Entries must be sorted by `data_offset` ascending. The first entry must begin at or after `first_asset_offset`; each following entry must begin at or after the previous entry's end. A loader may scan the entry table once and then stream payloads in table order.

### Asset Type IDs

| ID  | Name     | Manifest `type` | Notes                                                  |
| :-- | :------- | :-------------- | :----------------------------------------------------- |
| 1   | bytecode | `bytecode`      | Runtime entry code, usually the manifest `entry`.      |
| 2   | image    | `image`         | Encoded image payload. Exact codec is engine-specific. |
| 3   | audio    | `audio`         | MML, PCM, or encoded audio payload.                    |
| 4   | font     | `font`          | Bitmap font or glyph table payload.                    |
| 5   | data     | `data`          | Generic app data.                                      |

Unknown asset types are allowed only when the manifest and runtime both declare support for them. The shell may still list such packages but must not launch them through an unsupported runtime.

### Entry Flags

| Bit  | Name       | Meaning                                                                      |
| :--- | :--------- | :--------------------------------------------------------------------------- |
| 0    | sequential | The payload is intended for forward-only streaming.                          |
| 1    | preload    | The loader should consider preloading this payload into PSRAM before launch. |
| 2    | entry      | This payload is the runtime entry asset named by the manifest `entry` field. |
| 3-31 | reserved   | Must be zero in v1.                                                          |

## String Table

The string table contains entry paths as UTF-8 byte strings. Paths are stored without trailing NUL bytes; `path_offset` and `path_len` identify each string.

Paths must use `/` separators, must be relative, and must not contain empty segments, `.`, `..`, drive prefixes, or leading `/`. The path namespace is the app package namespace, not the host filesystem namespace.

## Manifest Relationship

The source manifest is JSON with `format: "kpa-manifest"`. Manifest version 2
adds the bounded Fetch permission below; version 1 remains loadable for existing
offline packages. The binary container remains KPA v1. The packer copies a
canonical UTF-8 JSON representation into the metadata region so loaders and
diagnostics can inspect app metadata without external files.

Network access is default-denied. A version 2 manifest may grant only a fixed
list of canonical origins:

```json
"permissions": {
  "network": {
    "origins": ["https://api.example.com", "https://data.example.net:8443"]
  }
}
```

An HTTPS origin that is intended for a future device transport uses an object
entry containing one current pin and optionally one next pin. Pins are the
lower-case hexadecimal SHA-256 digest of the certificate's Subject Public Key
Info (SPKI):

```json
"origins": [
  {
    "origin": "https://api.example.com",
    "spki_sha256": [
      "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
      "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
    ]
  }
]
```

Each digest is exactly 32 bytes represented by 64 lower-case hexadecimal
characters. A pin set contains one or two distinct values and is permitted only
for HTTPS. The first value is current and the second is the staged successor;
either value authenticates the peer. Rotation requires installing a signed KPA
update that overlaps old and new keys, followed by another signed update that
removes the old key. Response data can never modify pins. String origin entries
remain valid for deterministic KotoSim and offline packages, but an HTTPS
device backend must remain `Unavailable` unless every requested HTTPS origin
has a non-empty pin set.

At most four exact `(scheme, hostname, port)` origins are accepted. Schemes are
`https` and development-only `http`; DNS hostnames must be lower-case ASCII.
Wildcards, user-info, IP literals, paths, queries, fragments, duplicates,
explicit default ports, malformed/oversized names, and other fields are
rejected. Absence or an empty list grants no access. Version 1's boolean
`permissions.network` is legacy metadata and never grants Fetch access.
KotoSim and the device catalog loader both apply the portable fixed-depth,
allocation-free permission parser to the complete metadata JSON. Duplicate
root permission declarations, duplicate network declarations, escaped origin
text, excessive nesting, and malformed JSON are rejected before launch. Device
builds retain no Fetch transport state until the authenticated backend is
enabled, so a validated declaration does not by itself make networking
available.

Manifest fields map into KPA v1 as follows:

| Manifest field                                               | KPA mapping                                                              |
| :----------------------------------------------------------- | :----------------------------------------------------------------------- |
| `app_id`, `name`, `runtime`, `memory`, `permissions`, `icon`, `shell_icon` | Stored in the metadata JSON copy.                              |
| `description`, `category`                                    | Optional launcher metadata stored in the metadata JSON copy.             |
| `entry`                                                      | Must match exactly one asset path; that entry receives the `entry` flag. |
| `icon`                                                       | Optional launcher icon path; when present, it must match one image asset. |
| `assets[].path`                                              | Stored in the string table and entry table.                              |
| `assets[].type`                                              | Converted to the asset type ID.                                          |
| `assets[].sequential`                                        | Converted to the `sequential` entry flag.                                |
| `assets[].preload`                                           | Optional; when true, converted to the `preload` entry flag.               |

### Launcher Metadata

The optional top-level `description` and `category` string fields give KotoShell
text for its home-screen details pane and category navigation. `description` is a
short summary (at most 128 UTF-8 bytes); `category` is a short grouping label (at
most 32 UTF-8 bytes). Both reject control characters. When absent, the shell
falls back to deterministic placeholders.

The v1 header intentionally does not duplicate launcher-facing strings such as `name` or `app_id`. Early loaders may read the small metadata JSON copy for package listing. KOTO-0011 will define shared validation rules for these manifest fields.

### Launcher Icon

The optional top-level `icon` field identifies the image asset KotoShell should use in package grids and launchers. The path uses the same package-relative path rules as `entry`, and the referenced asset should use manifest `type: "image"` so it is stored as an image entry in the KPA table.

KotoShell currently accepts `KICON1`, a simple text 1-bit bitmap format: ASCII `KICON1` on the first line, followed by exactly 40 lines of 40 pixels each, where `#` means foreground and `.` means background. KotoShell-friendly packages should keep launcher icons small and directly decodable; 40x40 1-bit bitmap, RGB565, or indexed-color payloads are preferred over PNG/JPEG-style compressed formats on RP2040-class hardware.

The optional `shell_icon` object controls launcher coloring without hard-coding
application IDs in KotoShell. It requires the generic `mask` style and six
`#RRGGBB` colors: `background`, `primary`, `secondary`,
`accent`, `highlight`, and `shadow`. `mask` colorizes the referenced `KICON1`
silhouette; the other styles are reusable shell compositions whose complete
palette comes from the manifest.

Example source manifest fragment:

```json
{
  "icon": "icons/app.kicon",
  "shell_icon": {
    "style": "mask",
    "background": "#FAE8B4",
    "primary": "#223052",
    "secondary": "#FFFFFF",
    "accent": "#C4302A",
    "highlight": "#FADC96",
    "shadow": "#D2972A"
  },
  "assets": [
    {
      "path": "bytecode/main.kbc",
      "type": "bytecode",
      "sequential": true
    },
    {
      "path": "icons/app.kicon",
      "type": "image",
      "sequential": false,
      "preload": true
    }
  ]
}
```

## Asset Ordering And Alignment

The packer must emit assets in manifest order unless a later manifest version adds an explicit ordering field. Manifest order is therefore the stable read order and the deterministic build order.

Payload alignment rules:

- `first_asset_offset` must be aligned to 4096 bytes.
- Every payload must start on at least a 512-byte boundary.
- Payloads marked `sequential` should be placed back-to-back in the same manifest order, with only required alignment padding between them.
- Bytecode should appear before large media assets so the runtime can launch after a short initial read.
- Padding bytes must be zero.

These rules favor SD card sector reads while keeping the format simple enough for a small embedded loader.

## Sequential-Read Constraints

KPA v1 assumes random SD access is expensive. Runtime and engine loaders must follow these constraints:

- Read the header and entry table first, then prefer payload access in ascending `data_offset` order.
- Do not require seeking backward within an asset stream.
- Use bounded SRAM windows for payload reads; do not require a full asset in SRAM.
- Treat PSRAM as an explicit preload or cache target through block-transfer APIs, not as pointer-addressable memory.
- If an engine needs random lookup into a large logical asset, the packer should create a small index asset followed by sequential data chunks.
- A package is invalid if an asset marked `sequential` overlaps another asset or appears before a lower-offset sequential dependency named earlier in the manifest.

## Loader Validation

A minimal loader must validate:

- Header magic, version, sizes, reserved bytes, and `package_size`.
- Metadata and table ranges are inside the file and do not overlap incorrectly.
- Entry records are sorted by payload offset and remain inside the file.
- Path strings are valid UTF-8 package paths.
- Reserved flag bits are zero.
- Exactly one entry has the `entry` flag and it matches the manifest `entry` value.
- Assets marked `sequential` are readable in entry order without backward seeks.

## Future Packer Requirements

KOTO-0020 should use this specification as the first target. The packer should:

- Read `*.kpa.json` manifests and referenced asset files.
- Validate manifest fields with the shared KOTO-0011 core rules.
- Emit deterministic KPA v1 binaries with stable ordering, zero padding, and little-endian tables.
- Produce a dry-run layout report showing offsets, sizes, alignment padding, and flags.
- Reject paths that escape the package namespace.
- Optionally compute and fill `content_hash`; until then it must write zeroes.
- Verify that payload offsets are monotonic and sequential assets are contiguous except for required alignment padding.
- Preserve the canonical manifest JSON copy inside the archive.
- Include fixture-based tests for byte-for-byte deterministic output.

## Open Questions For Later Versions

- Whether `app_id` and `name` should be duplicated in fixed-size header fields for faster shell listing.
- Which image and audio codecs should receive dedicated type IDs.
- Whether compressed payloads need explicit uncompressed-size fields.
- Whether per-entry checksums are worth the loader cost on RP2040.
