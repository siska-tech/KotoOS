use std::fmt;
use std::path::{Path, PathBuf};

use koto_audio::{runtime_cue_max_encoded_len, RuntimeCue, RuntimeCueError};
use koto_core::package::validate_entry_path;
use koto_core::{ManifestFields, PackageIconStyle, PackageIconTheme, PackageManifest};
use serde::Deserialize;

const HEADER_SIZE: u32 = 64;
const ENTRY_SIZE: u32 = 64;
const FIRST_ASSET_ALIGNMENT: u32 = 4096;
const PAYLOAD_ALIGNMENT: u32 = 512;
const FLAG_SEQUENTIAL: u32 = 1 << 0;
const FLAG_PRELOAD: u32 = 1 << 1;
const FLAG_ENTRY: u32 = 1 << 2;

#[derive(Clone, Debug)]
pub struct PackOptions {
    pub assets_root: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackedPackage {
    bytes: Vec<u8>,
    layout: Vec<LayoutEntry>,
}

impl PackedPackage {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn layout(&self) -> &[LayoutEntry] {
        &self.layout
    }

    pub fn layout_report(&self) -> String {
        let mut report = String::from("path,type,offset,size,alignment,flags,padding_before\n");
        for entry in &self.layout {
            report.push_str(&format!(
                "{},{},{},{},{},{},{}\n",
                entry.path,
                entry.asset_type,
                entry.data_offset,
                entry.data_size,
                entry.alignment,
                entry.flags_text(),
                entry.padding_before
            ));
        }
        report
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LayoutEntry {
    pub path: String,
    pub asset_type: String,
    pub data_offset: u32,
    pub data_size: u32,
    pub alignment: u32,
    pub flags: u32,
    pub padding_before: u32,
}

impl LayoutEntry {
    fn flags_text(&self) -> String {
        let mut parts = Vec::new();
        if self.flags & FLAG_SEQUENTIAL != 0 {
            parts.push("sequential");
        }
        if self.flags & FLAG_PRELOAD != 0 {
            parts.push("preload");
        }
        if self.flags & FLAG_ENTRY != 0 {
            parts.push("entry");
        }
        if parts.is_empty() {
            "none".to_string()
        } else {
            parts.join("|")
        }
    }
}

#[derive(Debug)]
pub enum PackError {
    ManifestJson(serde_json::Error),
    ManifestFields,
    InvalidAssetPath(String),
    EntryAssetMissing(String),
    AssetRead {
        path: String,
        source: std::io::Error,
    },
    AudioCompile {
        path: String,
        source: RuntimeCueError,
    },
    Layout(LayoutError),
    SizeOverflow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LayoutError {
    NonMonotonicAsset {
        previous_path: String,
        path: String,
        previous_end: u32,
        offset: u32,
    },
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonMonotonicAsset {
                previous_path,
                path,
                previous_end,
                offset,
            } => write!(
                f,
                "asset {path} starts at {offset}, before previous asset {previous_path} ends at {previous_end}"
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ManifestJson(error) => write!(f, "invalid manifest JSON: {error}"),
            Self::ManifestFields => write!(f, "manifest fields failed core validation"),
            Self::InvalidAssetPath(path) => write!(f, "invalid package asset path: {path}"),
            Self::EntryAssetMissing(path) => {
                write!(f, "manifest entry does not match exactly one asset: {path}")
            }
            Self::AssetRead { path, source } => {
                write!(f, "failed to read asset {path}: {source}")
            }
            Self::AudioCompile { path, source } => {
                write!(
                    f,
                    "failed to compile Native KotoAudio asset {path}: {source:?}"
                )
            }
            Self::Layout(error) => write!(f, "invalid package layout: {error}"),
            Self::SizeOverflow => write!(f, "package exceeds KPA v1 32-bit size fields"),
        }
    }
}

impl std::error::Error for PackError {}

impl From<LayoutError> for PackError {
    fn from(value: LayoutError) -> Self {
        Self::Layout(value)
    }
}

pub fn pack_manifest(
    manifest_bytes: &[u8],
    options: PackOptions,
) -> Result<PackedPackage, PackError> {
    let manifest: SourceManifest =
        serde_json::from_slice(manifest_bytes).map_err(PackError::ManifestJson)?;
    let shell_icon = manifest
        .shell_icon
        .as_ref()
        .map(SourceShellIcon::to_theme)
        .transpose()
        .map_err(|_| PackError::ManifestFields)?;
    PackageManifest::new(ManifestFields {
        format: &manifest.format,
        version: manifest.version,
        app_id: &manifest.app_id,
        name: &manifest.name,
        runtime: &manifest.runtime,
        entry: &manifest.entry,
        icon: manifest.icon.as_deref(),
        shell_icon,
        fs_permission: None,
        network_permission: None,
        sram_work_bytes: None,
        psram_cache_bytes: None,
        description: manifest.description.as_deref(),
        category: manifest.category.as_deref(),
    })
    .map_err(|_| PackError::ManifestFields)?;

    let metadata_value: serde_json::Value =
        serde_json::from_slice(manifest_bytes).map_err(PackError::ManifestJson)?;
    let metadata = serde_json::to_string(&metadata_value)
        .map_err(PackError::ManifestJson)?
        .into_bytes();

    let assets = manifest
        .assets
        .iter()
        .map(|asset| read_asset(asset, &manifest.entry, &options.assets_root))
        .collect::<Result<Vec<_>, _>>()?;

    let entry_matches = assets
        .iter()
        .filter(|asset| asset.manifest.path == manifest.entry)
        .count();
    if entry_matches != 1 {
        return Err(PackError::EntryAssetMissing(manifest.entry));
    }

    let package = build_package(&assets, &metadata)?;
    validate_layout(package.layout())?;
    Ok(package)
}

pub fn validate_layout(layout: &[LayoutEntry]) -> Result<(), LayoutError> {
    let mut previous: Option<&LayoutEntry> = None;
    for entry in layout {
        if let Some(previous_entry) = previous {
            let previous_end = previous_entry
                .data_offset
                .saturating_add(previous_entry.data_size);
            if entry.data_offset < previous_end {
                return Err(LayoutError::NonMonotonicAsset {
                    previous_path: previous_entry.path.clone(),
                    path: entry.path.clone(),
                    previous_end,
                    offset: entry.data_offset,
                });
            }
        }
        previous = Some(entry);
    }
    Ok(())
}

fn read_asset(
    asset: &SourceAsset,
    manifest_entry: &str,
    assets_root: &Path,
) -> Result<AssetInput, PackError> {
    validate_entry_path(&asset.path)
        .map_err(|_| PackError::InvalidAssetPath(asset.path.clone()))?;
    let host_path = host_asset_path(assets_root, &asset.path);
    let mut bytes = std::fs::read(&host_path).map_err(|source| PackError::AssetRead {
        path: asset.path.clone(),
        source,
    })?;
    if asset.asset_type == "audio" && !bytes.starts_with(b"KAQ1") && !bytes.starts_with(b"KACL") {
        let source = std::str::from_utf8(&bytes).map_err(|_| PackError::AudioCompile {
            path: asset.path.clone(),
            source: RuntimeCueError::InvalidText,
        })?;
        let cue =
            RuntimeCue::<272>::compile_kmml(source).map_err(|source| PackError::AudioCompile {
                path: asset.path.clone(),
                source,
            })?;
        let mut image = vec![0u8; runtime_cue_max_encoded_len::<272>()];
        let len = cue
            .encode(&mut image)
            .map_err(|source| PackError::AudioCompile {
                path: asset.path.clone(),
                source,
            })?;
        image.truncate(len);
        bytes = image;
    }
    let mut flags = 0;
    if asset.sequential {
        flags |= FLAG_SEQUENTIAL;
    }
    if asset.preload {
        flags |= FLAG_PRELOAD;
    }
    if asset.path == manifest_entry {
        flags |= FLAG_ENTRY;
    }
    Ok(AssetInput {
        manifest: asset.clone(),
        bytes,
        flags,
    })
}

fn host_asset_path(root: &Path, package_path: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for segment in package_path.split('/') {
        path.push(segment);
    }
    path
}

fn build_package(assets: &[AssetInput], metadata: &[u8]) -> Result<PackedPackage, PackError> {
    let entry_count = checked_u32(assets.len())?;
    let table_offset = HEADER_SIZE;
    let string_table_offset = table_offset + ENTRY_SIZE * entry_count;
    let mut strings = Vec::new();
    let mut string_ranges = Vec::new();
    for asset in assets {
        let offset = checked_u32(strings.len())?;
        strings.extend_from_slice(asset.manifest.path.as_bytes());
        string_ranges.push((offset, checked_u32(asset.manifest.path.len())?));
    }
    let string_table_size = checked_u32(strings.len())?;
    let metadata_offset = string_table_offset + string_table_size;
    let metadata_size = checked_u32(metadata.len())?;
    let first_asset_offset = align_up(metadata_offset + metadata_size, FIRST_ASSET_ALIGNMENT)?;

    let mut layout = Vec::with_capacity(assets.len());
    let mut cursor = first_asset_offset;
    for asset in assets {
        let data_offset = align_up(cursor, PAYLOAD_ALIGNMENT)?;
        let data_size = checked_u32(asset.bytes.len())?;
        layout.push(LayoutEntry {
            path: asset.manifest.path.clone(),
            asset_type: asset.manifest.asset_type.clone(),
            data_offset,
            data_size,
            alignment: PAYLOAD_ALIGNMENT,
            flags: asset.flags,
            padding_before: data_offset - cursor,
        });
        cursor = data_offset
            .checked_add(data_size)
            .ok_or(PackError::SizeOverflow)?;
    }
    let package_size = cursor;

    let mut bytes = Vec::new();
    write_header(
        &mut bytes,
        HeaderFields {
            entry_count,
            string_table_offset,
            string_table_size,
            metadata_offset,
            metadata_size,
            first_asset_offset,
            package_size,
        },
    );
    for ((asset, (path_offset, path_len)), layout_entry) in assets
        .iter()
        .zip(string_ranges.iter().copied())
        .zip(layout.iter())
    {
        write_entry(
            &mut bytes,
            path_offset,
            path_len,
            asset_type_id(&asset.manifest.asset_type),
            layout_entry.flags,
            layout_entry.data_offset,
            layout_entry.data_size,
        );
    }
    bytes.extend_from_slice(&strings);
    bytes.extend_from_slice(metadata);
    pad_to(&mut bytes, first_asset_offset)?;
    for (asset, layout_entry) in assets.iter().zip(layout.iter()) {
        pad_to(&mut bytes, layout_entry.data_offset)?;
        bytes.extend_from_slice(&asset.bytes);
    }

    Ok(PackedPackage { bytes, layout })
}

struct HeaderFields {
    entry_count: u32,
    string_table_offset: u32,
    string_table_size: u32,
    metadata_offset: u32,
    metadata_size: u32,
    first_asset_offset: u32,
    package_size: u32,
}

fn write_header(bytes: &mut Vec<u8>, fields: HeaderFields) {
    bytes.extend_from_slice(b"KPA1");
    push_u16(bytes, 1);
    push_u16(bytes, 0);
    push_u32(bytes, HEADER_SIZE);
    push_u32(bytes, 0);
    push_u32(bytes, fields.entry_count);
    push_u32(bytes, HEADER_SIZE);
    push_u32(bytes, fields.string_table_offset);
    push_u32(bytes, fields.string_table_size);
    push_u32(bytes, fields.metadata_offset);
    push_u32(bytes, fields.metadata_size);
    push_u32(bytes, fields.first_asset_offset);
    push_u32(bytes, fields.package_size);
    bytes.extend_from_slice(&[0; 16]);
}

fn write_entry(
    bytes: &mut Vec<u8>,
    path_offset: u32,
    path_len: u32,
    asset_type: u32,
    flags: u32,
    data_offset: u32,
    data_size: u32,
) {
    push_u32(bytes, path_offset);
    push_u32(bytes, path_len);
    push_u32(bytes, asset_type);
    push_u32(bytes, flags);
    push_u32(bytes, data_offset);
    push_u32(bytes, data_size);
    push_u32(bytes, PAYLOAD_ALIGNMENT);
    push_u32(bytes, 0);
    bytes.extend_from_slice(&[0; 32]);
}

fn asset_type_id(asset_type: &str) -> u32 {
    match asset_type {
        "bytecode" => 1,
        "image" => 2,
        "audio" => 3,
        "font" => 4,
        "data" => 5,
        _ => 5,
    }
}

fn align_up(value: u32, alignment: u32) -> Result<u32, PackError> {
    let addend = alignment - 1;
    let rounded = value.checked_add(addend).ok_or(PackError::SizeOverflow)?;
    Ok(rounded / alignment * alignment)
}

fn checked_u32(value: usize) -> Result<u32, PackError> {
    u32::try_from(value).map_err(|_| PackError::SizeOverflow)
}

fn pad_to(bytes: &mut Vec<u8>, target_len: u32) -> Result<(), PackError> {
    let target_len = usize::try_from(target_len).map_err(|_| PackError::SizeOverflow)?;
    if bytes.len() > target_len {
        return Err(PackError::SizeOverflow);
    }
    bytes.resize(target_len, 0);
    Ok(())
}

fn push_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[derive(Clone, Debug, Deserialize)]
struct SourceManifest {
    format: String,
    version: u32,
    app_id: String,
    name: String,
    runtime: String,
    entry: String,
    icon: Option<String>,
    #[serde(default)]
    shell_icon: Option<SourceShellIcon>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
    assets: Vec<SourceAsset>,
}

#[derive(Clone, Debug, Deserialize)]
struct SourceShellIcon {
    style: String,
    background: String,
    primary: String,
    secondary: String,
    accent: String,
    highlight: String,
    shadow: String,
}

impl SourceShellIcon {
    fn to_theme(&self) -> Result<PackageIconTheme, ()> {
        let style = match self.style.as_str() {
            "mask" => PackageIconStyle::Mask,
            _ => return Err(()),
        };
        Ok(PackageIconTheme {
            style,
            background: parse_rgb565(&self.background)?,
            primary: parse_rgb565(&self.primary)?,
            secondary: parse_rgb565(&self.secondary)?,
            accent: parse_rgb565(&self.accent)?,
            highlight: parse_rgb565(&self.highlight)?,
            shadow: parse_rgb565(&self.shadow)?,
        })
    }
}

fn parse_rgb565(text: &str) -> Result<u16, ()> {
    let hex = text.strip_prefix('#').ok_or(())?;
    if hex.len() != 6 {
        return Err(());
    }
    let rgb = u32::from_str_radix(hex, 16).map_err(|_| ())?;
    let r = (rgb >> 16) & 0xff;
    let g = (rgb >> 8) & 0xff;
    let b = rgb & 0xff;
    Ok((((r * 31 / 255) << 11) | ((g * 63 / 255) << 5) | (b * 31 / 255)) as u16)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct SourceAsset {
    path: String,
    #[serde(rename = "type")]
    asset_type: String,
    sequential: bool,
    #[serde(default)]
    preload: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AssetInput {
    manifest: SourceAsset,
    bytes: Vec<u8>,
    flags: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("harness/fixtures")
    }

    #[test]
    fn fixture_manifest_packs_deterministically() {
        let root = fixture_root();
        let manifest = std::fs::read(root.join("sample_app.kpa.json")).unwrap();
        let options = PackOptions {
            assets_root: root.join("package_assets"),
        };

        let first = pack_manifest(&manifest, options.clone()).unwrap();
        let second = pack_manifest(&manifest, options).unwrap();

        assert_eq!(first.bytes(), second.bytes());
        assert_eq!(&first.bytes()[..4], b"KPA1");
        assert_eq!(first.layout[0].path, "bytecode/main.kbc");
        assert_eq!(first.layout[1].path, "assets/title.rle");
        assert_eq!(first.layout[2].path, "icons/sample.kicon");
        assert_eq!(first.layout[0].data_offset, 4096);
        assert_eq!(first.layout[1].data_offset % 512, 0);
    }

    #[test]
    fn dry_run_report_preserves_manifest_order() {
        let root = fixture_root();
        let manifest = std::fs::read(root.join("sample_app.kpa.json")).unwrap();
        let package = pack_manifest(
            &manifest,
            PackOptions {
                assets_root: root.join("package_assets"),
            },
        )
        .unwrap();

        let report = package.layout_report();
        let bytecode = report.find("bytecode/main.kbc").unwrap();
        let title = report.find("assets/title.rle").unwrap();
        let icon = report.find("icons/sample.kicon").unwrap();

        assert!(bytecode < title);
        assert!(title < icon);
        assert!(report.contains("sequential|entry"));
    }

    #[test]
    fn layout_validation_detects_non_monotonic_offsets() {
        let layout = vec![
            LayoutEntry {
                path: "assets/first.bin".to_string(),
                asset_type: "data".to_string(),
                data_offset: 4096,
                data_size: 128,
                alignment: 512,
                flags: 0,
                padding_before: 0,
            },
            LayoutEntry {
                path: "assets/second.bin".to_string(),
                asset_type: "data".to_string(),
                data_offset: 4000,
                data_size: 64,
                alignment: 512,
                flags: 0,
                padding_before: 0,
            },
        ];

        assert!(matches!(
            validate_layout(&layout),
            Err(LayoutError::NonMonotonicAsset { .. })
        ));
    }

    #[test]
    fn audio_assets_are_compiled_to_pointer_free_kaq1_images() {
        let root = std::env::temp_dir().join(format!("kpa-packer-audio-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bytecode")).unwrap();
        std::fs::create_dir_all(root.join("audio")).unwrap();
        std::fs::write(root.join("bytecode/main.kbc"), b"KBC").unwrap();
        std::fs::write(
            root.join("audio/theme.kmml"),
            b"#DIALECT KOTOAUDIO\n#TRACK drums\nT120 L8 [!bd !hh !sd]0\n",
        )
        .unwrap();
        let manifest = br#"{
            "format":"kpa-manifest","version":1,"app_id":"dev.koto.audio",
            "name":"Audio","runtime":"kotoruntime-bytecode",
            "entry":"bytecode/main.kbc",
            "assets":[
                {"path":"bytecode/main.kbc","type":"bytecode","sequential":true},
                {"path":"audio/theme.kmml","type":"audio","sequential":true}
            ]
        }"#;
        let package = pack_manifest(
            manifest,
            PackOptions {
                assets_root: root.clone(),
            },
        )
        .unwrap();
        let audio = &package.layout()[1];
        assert_eq!(
            &package.bytes()[audio.data_offset as usize..audio.data_offset as usize + 4],
            b"KAQ1"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_ready_kacl_audio_is_packaged_byte_for_byte() {
        let root = std::env::temp_dir().join(format!("kpa-packer-kacl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bytecode")).unwrap();
        std::fs::create_dir_all(root.join("audio")).unwrap();
        std::fs::write(root.join("bytecode/main.kbc"), b"KBC").unwrap();
        let kacl = b"KACLruntime-ready-payload";
        std::fs::write(root.join("audio/clip.kacl"), kacl).unwrap();
        let manifest = br#"{
            "format":"kpa-manifest","version":1,"app_id":"dev.koto.kacl",
            "name":"KACL","runtime":"kotoruntime-bytecode",
            "entry":"bytecode/main.kbc",
            "assets":[
                {"path":"bytecode/main.kbc","type":"bytecode","sequential":true},
                {"path":"audio/clip.kacl","type":"audio","sequential":true}
            ]
        }"#;
        let package = pack_manifest(
            manifest,
            PackOptions {
                assets_root: root.clone(),
            },
        )
        .unwrap();
        let audio = &package.layout()[1];
        let start = audio.data_offset as usize;
        assert_eq!(&package.bytes()[start..start + kacl.len()], kacl);
        let _ = std::fs::remove_dir_all(root);
    }
}
