use koto_core::{ManifestFields, PackageIconStyle, PackageIconTheme, PackageInfo, PackageManifest};
use serde_json::Value;

use crate::SimError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageLaunch {
    pub package: PackageInfo,
    pub(crate) runtime: String,
    pub(crate) entry: String,
    pub(crate) asset_paths: Vec<String>,
}

impl PackageLaunch {
    pub fn runtime(&self) -> &str {
        &self.runtime
    }

    pub fn entry(&self) -> &str {
        &self.entry
    }

    pub fn asset_paths(&self) -> &[String] {
        &self.asset_paths
    }
}

pub fn parse_manifest(text: &str) -> Result<PackageInfo, SimError> {
    parse_launch_manifest(text).map(|launch| launch.package)
}

pub fn parse_launch_manifest(text: &str) -> Result<PackageLaunch, SimError> {
    let document: Value = serde_json::from_str(text).map_err(|_| SimError::InvalidManifest)?;
    let root = document.as_object().ok_or(SimError::InvalidManifest)?;

    let format = required_string(root.get("format"))?;
    let version = required_u32(root.get("version"))?;
    let app_id = required_string(root.get("app_id"))?;
    let name = required_string(root.get("name"))?;
    let runtime = required_string(root.get("runtime"))?;
    let entry = required_string(root.get("entry"))?;
    let icon = optional_string(root.get("icon"));
    let shell_icon = parse_shell_icon(root.get("shell_icon"))?;
    let description = optional_string(root.get("description"));
    let category = optional_string(root.get("category"));
    let asset_paths = root
        .get("assets")
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .filter_map(|asset| asset.get("path"))
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let permissions = root.get("permissions").and_then(Value::as_object);
    let fs_permission = optional_string(permissions.and_then(|value| value.get("fs")));
    let network_permission = permissions
        .and_then(|value| value.get("network"))
        .and_then(Value::as_bool);

    let memory = root.get("memory").and_then(Value::as_object);
    let sram_work_bytes = optional_u32(memory.and_then(|value| value.get("sram_work_bytes")));
    let psram_cache_bytes = optional_u32(memory.and_then(|value| value.get("psram_cache_bytes")));

    PackageManifest::new(ManifestFields {
        format: &format,
        version,
        app_id: &app_id,
        name: &name,
        runtime: &runtime,
        entry: &entry,
        icon: icon.as_deref(),
        shell_icon,
        fs_permission: fs_permission.as_deref(),
        network_permission,
        sram_work_bytes,
        psram_cache_bytes,
        description: description.as_deref(),
        category: category.as_deref(),
    })
    .map(|manifest| PackageLaunch {
        package: manifest.package(),
        runtime,
        entry,
        asset_paths,
    })
    .map_err(|_| SimError::InvalidManifest)
}

fn parse_shell_icon(value: Option<&Value>) -> Result<Option<PackageIconTheme>, SimError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let object = value.as_object().ok_or(SimError::InvalidManifest)?;
    let style = match required_string(object.get("style"))?.as_str() {
        "mask" => PackageIconStyle::Mask,
        _ => return Err(SimError::InvalidManifest),
    };
    Ok(Some(PackageIconTheme {
        style,
        background: required_rgb565(object.get("background"))?,
        primary: required_rgb565(object.get("primary"))?,
        secondary: required_rgb565(object.get("secondary"))?,
        accent: required_rgb565(object.get("accent"))?,
        highlight: required_rgb565(object.get("highlight"))?,
        shadow: required_rgb565(object.get("shadow"))?,
    }))
}

fn required_rgb565(value: Option<&Value>) -> Result<u16, SimError> {
    let text = value
        .and_then(Value::as_str)
        .ok_or(SimError::InvalidManifest)?;
    let hex = text.strip_prefix('#').ok_or(SimError::InvalidManifest)?;
    if hex.len() != 6 {
        return Err(SimError::InvalidManifest);
    }
    let rgb = u32::from_str_radix(hex, 16).map_err(|_| SimError::InvalidManifest)?;
    let r = (rgb >> 16) & 0xff;
    let g = (rgb >> 8) & 0xff;
    let b = rgb & 0xff;
    Ok((((r * 31 / 255) << 11) | ((g * 63 / 255) << 5) | (b * 31 / 255)) as u16)
}

fn required_string(value: Option<&Value>) -> Result<String, SimError> {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or(SimError::InvalidManifest)
}

fn optional_string(value: Option<&Value>) -> Option<String> {
    value.and_then(Value::as_str).map(str::to_string)
}

fn required_u32(value: Option<&Value>) -> Result<u32, SimError> {
    optional_u32(value).ok_or(SimError::InvalidManifest)
}

fn optional_u32(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_json_before_core_validation() {
        assert_eq!(
            parse_manifest(r#"{"format":"kpa-manifest","version":1"#),
            Err(SimError::InvalidManifest)
        );
    }

    #[test]
    fn does_not_confuse_nested_keys_with_required_root_fields() {
        let manifest = r#"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.structured",
            "name": "Structured",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc",
            "metadata": { "name": "Wrong nested name" }
        }"#;

        let package = parse_manifest(manifest).unwrap();
        assert_eq!(package.name(), "Structured");
    }

    #[test]
    fn parses_manifest_driven_shell_icon_theme() {
        let manifest = r##"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.icon-theme",
            "name": "Icon Theme",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc",
            "icon": "icons/theme.kicon",
            "shell_icon": {
                "style": "mask",
                "background": "#1A263E",
                "primary": "#2ACADC",
                "secondary": "#F6C236",
                "accent": "#B058D2",
                "highlight": "#46BE69",
                "shadow": "#0E1626"
            }
        }"##;

        let package = parse_manifest(manifest).unwrap();
        let theme = package.shell_icon().unwrap();
        assert_eq!(theme.style, PackageIconStyle::Mask);
        assert_ne!(theme.background, theme.primary);
        assert_ne!(theme.accent, theme.highlight);
    }

    #[test]
    fn rejects_unknown_shell_icon_style() {
        let manifest = r##"{
            "format": "kpa-manifest",
            "version": 1,
            "app_id": "dev.koto.bad-icon-theme",
            "name": "Bad Icon Theme",
            "runtime": "kotoruntime-bytecode",
            "entry": "bytecode/main.kbc",
            "shell_icon": {
                "style": "per-app-special-case",
                "background": "#000000",
                "primary": "#000000",
                "secondary": "#000000",
                "accent": "#000000",
                "highlight": "#000000",
                "shadow": "#000000"
            }
        }"##;
        assert_eq!(parse_manifest(manifest), Err(SimError::InvalidManifest));
    }
}
