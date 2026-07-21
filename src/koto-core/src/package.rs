pub const MAX_PACKAGES: usize = 32;
pub const MAX_APP_ID_LEN: usize = 64;
pub const MAX_NAME_LEN: usize = 64;
pub const MAX_RUNTIME_NAME_LEN: usize = 32;
pub const MAX_ENTRY_PATH_LEN: usize = 128;
pub const MAX_ICON_PATH_LEN: usize = MAX_ENTRY_PATH_LEN;
pub const MAX_PERMISSION_VALUE_LEN: usize = 32;
pub const MAX_DESCRIPTION_LEN: usize = 128;
pub const MAX_CATEGORY_LEN: usize = 32;
pub const PACKAGE_ICON_WIDTH: usize = 40;
pub const PACKAGE_ICON_HEIGHT: usize = 40;
pub const PACKAGE_ICON_1BPP_BYTES: usize = (PACKAGE_ICON_WIDTH * PACKAGE_ICON_HEIGHT) / 8;
pub const KPA_MANIFEST_FORMAT: &str = "kpa-manifest";
pub const KPA_MANIFEST_VERSION: u32 = 2;
pub const KPA_MANIFEST_MIN_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    InvalidFormat,
    UnsupportedVersion,
    InvalidAppId,
    InvalidName,
    InvalidRuntime,
    InvalidEntry,
    InvalidIcon,
    InvalidDescription,
    InvalidCategory,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconError {
    InvalidHeader,
    InvalidSize,
    InvalidPixel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageIconStyle {
    Mask,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageIconTheme {
    pub style: PackageIconStyle,
    pub background: u16,
    pub primary: u16,
    pub secondary: u16,
    pub accent: u16,
    pub highlight: u16,
    pub shadow: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageIcon {
    pixels: [u8; PACKAGE_ICON_1BPP_BYTES],
}

impl PackageIcon {
    pub const fn from_1bpp_pixels(pixels: [u8; PACKAGE_ICON_1BPP_BYTES]) -> Self {
        Self { pixels }
    }

    pub fn from_kicon_text(bytes: &[u8]) -> Result<Self, IconError> {
        let text = core::str::from_utf8(bytes).map_err(|_| IconError::InvalidHeader)?;
        let mut lines = text.lines();
        if lines.next() != Some("KICON1") {
            return Err(IconError::InvalidHeader);
        }

        let mut pixels = [0u8; PACKAGE_ICON_1BPP_BYTES];
        for y in 0..PACKAGE_ICON_HEIGHT {
            let line = lines.next().ok_or(IconError::InvalidSize)?;
            if line.len() != PACKAGE_ICON_WIDTH {
                return Err(IconError::InvalidSize);
            }
            for (x, byte) in line.bytes().enumerate() {
                match byte {
                    b'#' => set_icon_bit(&mut pixels, x, y),
                    b'.' => {}
                    _ => return Err(IconError::InvalidPixel),
                }
            }
        }
        Ok(Self { pixels })
    }

    pub fn pixel(&self, x: usize, y: usize) -> bool {
        if x >= PACKAGE_ICON_WIDTH || y >= PACKAGE_ICON_HEIGHT {
            return false;
        }
        let bit = y * PACKAGE_ICON_WIDTH + x;
        (self.pixels[bit / 8] & (0x80 >> (bit % 8))) != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManifestFields<'a> {
    pub format: &'a str,
    pub version: u32,
    pub app_id: &'a str,
    pub name: &'a str,
    pub runtime: &'a str,
    pub entry: &'a str,
    pub icon: Option<&'a str>,
    pub shell_icon: Option<PackageIconTheme>,
    pub fs_permission: Option<&'a str>,
    pub network_permission: Option<bool>,
    pub sram_work_bytes: Option<u32>,
    pub psram_cache_bytes: Option<u32>,
    pub description: Option<&'a str>,
    pub category: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageManifest {
    package: PackageInfo,
    runtime: [u8; MAX_RUNTIME_NAME_LEN],
    runtime_len: usize,
    entry: [u8; MAX_ENTRY_PATH_LEN],
    entry_len: usize,
}

impl PackageManifest {
    pub fn new(fields: ManifestFields<'_>) -> Result<Self, ManifestError> {
        if fields.format != KPA_MANIFEST_FORMAT {
            return Err(ManifestError::InvalidFormat);
        }
        if !(KPA_MANIFEST_MIN_VERSION..=KPA_MANIFEST_VERSION).contains(&fields.version) {
            return Err(ManifestError::UnsupportedVersion);
        }

        validate_app_id(fields.app_id)?;
        validate_display_name(fields.name)?;
        if let Some(icon) = fields.icon {
            validate_icon_path(icon)?;
        }
        let mut package = PackageInfo::new_with_icon(fields.app_id, fields.name, fields.icon)
            .expect("validated app_id and name should fit PackageInfo");

        validate_runtime_name(fields.runtime)?;
        validate_entry_path(fields.entry)?;
        package.set_runtime(fields.runtime);
        package.set_entry(fields.entry);
        if let Some(fs_permission) = fields.fs_permission {
            package.set_fs_permission(fs_permission)?;
        }
        if let Some(network_permission) = fields.network_permission {
            package.set_network_permission(network_permission);
        }
        if fields.sram_work_bytes.is_some() || fields.psram_cache_bytes.is_some() {
            package.set_memory_request(fields.sram_work_bytes, fields.psram_cache_bytes);
        }
        if let Some(description) = fields.description {
            package.set_description(description)?;
        }
        if let Some(category) = fields.category {
            package.set_category(category)?;
        }
        if let Some(shell_icon) = fields.shell_icon {
            package.set_shell_icon(shell_icon);
        }

        let mut manifest = Self {
            package,
            runtime: [0; MAX_RUNTIME_NAME_LEN],
            runtime_len: fields.runtime.len(),
            entry: [0; MAX_ENTRY_PATH_LEN],
            entry_len: fields.entry.len(),
        };
        manifest.runtime[..fields.runtime.len()].copy_from_slice(fields.runtime.as_bytes());
        manifest.entry[..fields.entry.len()].copy_from_slice(fields.entry.as_bytes());
        Ok(manifest)
    }

    pub fn package(self) -> PackageInfo {
        self.package
    }

    pub fn runtime(&self) -> &str {
        core::str::from_utf8(&self.runtime[..self.runtime_len]).unwrap_or("")
    }

    pub fn entry(&self) -> &str {
        core::str::from_utf8(&self.entry[..self.entry_len]).unwrap_or("")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageInfo {
    pub app_id: [u8; MAX_APP_ID_LEN],
    pub app_id_len: usize,
    pub name: [u8; MAX_NAME_LEN],
    pub name_len: usize,
    icon_path: [u8; MAX_ICON_PATH_LEN],
    icon_path_len: usize,
    icon: Option<PackageIcon>,
    shell_icon: Option<PackageIconTheme>,
    runtime: [u8; MAX_RUNTIME_NAME_LEN],
    runtime_len: usize,
    entry: [u8; MAX_ENTRY_PATH_LEN],
    entry_len: usize,
    fs_permission: [u8; MAX_PERMISSION_VALUE_LEN],
    fs_permission_len: usize,
    network_permission: Option<bool>,
    sram_work_bytes: Option<u32>,
    psram_cache_bytes: Option<u32>,
    description: [u8; MAX_DESCRIPTION_LEN],
    description_len: usize,
    category: [u8; MAX_CATEGORY_LEN],
    category_len: usize,
    save_data_present: bool,
    favorite: bool,
}

impl PackageInfo {
    pub fn new(app_id: &str, name: &str) -> Option<Self> {
        Self::new_with_icon(app_id, name, None)
    }

    pub fn new_with_icon(app_id: &str, name: &str, icon_path: Option<&str>) -> Option<Self> {
        if validate_app_id(app_id).is_err() || validate_display_name(name).is_err() {
            return None;
        }
        if let Some(path) = icon_path {
            if validate_icon_path(path).is_err() {
                return None;
            }
        }

        let mut info = Self {
            app_id: [0; MAX_APP_ID_LEN],
            app_id_len: app_id.len(),
            name: [0; MAX_NAME_LEN],
            name_len: name.len(),
            icon_path: [0; MAX_ICON_PATH_LEN],
            icon_path_len: icon_path.map(str::len).unwrap_or(0),
            icon: None,
            shell_icon: None,
            runtime: [0; MAX_RUNTIME_NAME_LEN],
            runtime_len: 0,
            entry: [0; MAX_ENTRY_PATH_LEN],
            entry_len: 0,
            fs_permission: [0; MAX_PERMISSION_VALUE_LEN],
            fs_permission_len: 0,
            network_permission: None,
            sram_work_bytes: None,
            psram_cache_bytes: None,
            description: [0; MAX_DESCRIPTION_LEN],
            description_len: 0,
            category: [0; MAX_CATEGORY_LEN],
            category_len: 0,
            save_data_present: false,
            favorite: false,
        };
        info.app_id[..app_id.len()].copy_from_slice(app_id.as_bytes());
        info.name[..name.len()].copy_from_slice(name.as_bytes());
        if let Some(path) = icon_path {
            info.icon_path[..path.len()].copy_from_slice(path.as_bytes());
        }
        Some(info)
    }

    pub fn app_id(&self) -> &str {
        core::str::from_utf8(&self.app_id[..self.app_id_len]).unwrap_or("")
    }

    pub fn name(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    pub fn icon_path(&self) -> Option<&str> {
        if self.icon_path_len == 0 {
            return None;
        }
        core::str::from_utf8(&self.icon_path[..self.icon_path_len]).ok()
    }

    pub fn icon(&self) -> Option<&PackageIcon> {
        self.icon.as_ref()
    }

    pub fn set_icon(&mut self, icon: PackageIcon) {
        self.icon = Some(icon);
    }

    pub fn shell_icon(&self) -> Option<PackageIconTheme> {
        self.shell_icon
    }

    pub fn set_shell_icon(&mut self, shell_icon: PackageIconTheme) {
        self.shell_icon = Some(shell_icon);
    }

    pub fn runtime(&self) -> Option<&str> {
        if self.runtime_len == 0 {
            return None;
        }
        core::str::from_utf8(&self.runtime[..self.runtime_len]).ok()
    }

    pub fn entry(&self) -> Option<&str> {
        if self.entry_len == 0 {
            return None;
        }
        core::str::from_utf8(&self.entry[..self.entry_len]).ok()
    }

    pub fn fs_permission(&self) -> Option<&str> {
        if self.fs_permission_len == 0 {
            return None;
        }
        core::str::from_utf8(&self.fs_permission[..self.fs_permission_len]).ok()
    }

    pub fn network_permission(&self) -> Option<bool> {
        self.network_permission
    }

    pub fn sram_work_bytes(&self) -> Option<u32> {
        self.sram_work_bytes
    }

    pub fn psram_cache_bytes(&self) -> Option<u32> {
        self.psram_cache_bytes
    }

    pub fn description(&self) -> Option<&str> {
        if self.description_len == 0 {
            return None;
        }
        core::str::from_utf8(&self.description[..self.description_len]).ok()
    }

    pub fn category(&self) -> Option<&str> {
        if self.category_len == 0 {
            return None;
        }
        core::str::from_utf8(&self.category[..self.category_len]).ok()
    }

    pub fn save_data_present(&self) -> bool {
        self.save_data_present
    }

    pub fn set_save_data_present(&mut self, present: bool) {
        self.save_data_present = present;
    }

    pub fn is_favorite(&self) -> bool {
        self.favorite
    }

    pub fn set_favorite(&mut self, favorite: bool) {
        self.favorite = favorite;
    }

    fn set_runtime(&mut self, runtime: &str) {
        self.runtime_len = runtime.len();
        self.runtime[..runtime.len()].copy_from_slice(runtime.as_bytes());
    }

    fn set_entry(&mut self, entry: &str) {
        self.entry_len = entry.len();
        self.entry[..entry.len()].copy_from_slice(entry.as_bytes());
    }

    fn set_fs_permission(&mut self, permission: &str) -> Result<(), ManifestError> {
        validate_permission_value(permission)?;
        self.fs_permission_len = permission.len();
        self.fs_permission[..permission.len()].copy_from_slice(permission.as_bytes());
        Ok(())
    }

    fn set_network_permission(&mut self, allowed: bool) {
        self.network_permission = Some(allowed);
    }

    fn set_memory_request(&mut self, sram_work_bytes: Option<u32>, psram_cache_bytes: Option<u32>) {
        self.sram_work_bytes = sram_work_bytes;
        self.psram_cache_bytes = psram_cache_bytes;
    }

    fn set_description(&mut self, description: &str) -> Result<(), ManifestError> {
        validate_description(description)?;
        self.description_len = description.len();
        self.description[..description.len()].copy_from_slice(description.as_bytes());
        Ok(())
    }

    fn set_category(&mut self, category: &str) -> Result<(), ManifestError> {
        validate_category(category)?;
        self.category_len = category.len();
        self.category[..category.len()].copy_from_slice(category.as_bytes());
        Ok(())
    }
}

pub fn validate_app_id(app_id: &str) -> Result<(), ManifestError> {
    if app_id.is_empty() || app_id.len() > MAX_APP_ID_LEN {
        return Err(ManifestError::InvalidAppId);
    }

    let mut segment_len = 0usize;
    let mut segment_start = true;
    let mut previous = None;

    for ch in app_id.bytes() {
        match ch {
            b'.' => {
                if segment_len == 0 || !is_ascii_lower_or_digit(previous.unwrap_or(0)) {
                    return Err(ManifestError::InvalidAppId);
                }
                segment_len = 0;
                segment_start = true;
            }
            b'a'..=b'z' if segment_start => {
                segment_len += 1;
                segment_start = false;
            }
            b'a'..=b'z' | b'0'..=b'9' | b'-' if !segment_start => {
                segment_len += 1;
            }
            _ => return Err(ManifestError::InvalidAppId),
        }
        previous = Some(ch);
    }

    if segment_len == 0 || !is_ascii_lower_or_digit(previous.unwrap_or(0)) {
        return Err(ManifestError::InvalidAppId);
    }

    Ok(())
}

pub fn validate_display_name(name: &str) -> Result<(), ManifestError> {
    if name.is_empty() || name.len() > MAX_NAME_LEN {
        return Err(ManifestError::InvalidName);
    }
    if name.chars().any(char::is_control) {
        return Err(ManifestError::InvalidName);
    }
    Ok(())
}

pub fn validate_description(description: &str) -> Result<(), ManifestError> {
    if description.len() > MAX_DESCRIPTION_LEN {
        return Err(ManifestError::InvalidDescription);
    }
    if description.chars().any(char::is_control) {
        return Err(ManifestError::InvalidDescription);
    }
    Ok(())
}

pub fn validate_category(category: &str) -> Result<(), ManifestError> {
    if category.len() > MAX_CATEGORY_LEN {
        return Err(ManifestError::InvalidCategory);
    }
    if category.chars().any(char::is_control) {
        return Err(ManifestError::InvalidCategory);
    }
    Ok(())
}

pub fn validate_runtime_name(runtime: &str) -> Result<(), ManifestError> {
    if runtime.is_empty() || runtime.len() > MAX_RUNTIME_NAME_LEN {
        return Err(ManifestError::InvalidRuntime);
    }

    let bytes = runtime.as_bytes();
    if !bytes[0].is_ascii_lowercase() {
        return Err(ManifestError::InvalidRuntime);
    }
    if !is_ascii_lower_or_digit(bytes[bytes.len() - 1]) {
        return Err(ManifestError::InvalidRuntime);
    }
    if !bytes
        .iter()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
    {
        return Err(ManifestError::InvalidRuntime);
    }

    Ok(())
}

pub fn validate_entry_path(path: &str) -> Result<(), ManifestError> {
    if path.is_empty() || path.len() > MAX_ENTRY_PATH_LEN || path.starts_with('/') {
        return Err(ManifestError::InvalidEntry);
    }

    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(ManifestError::InvalidEntry);
        }
        if segment.contains('\\') || segment.contains(':') {
            return Err(ManifestError::InvalidEntry);
        }
        if segment.chars().any(char::is_control) {
            return Err(ManifestError::InvalidEntry);
        }
    }

    Ok(())
}

pub fn validate_icon_path(path: &str) -> Result<(), ManifestError> {
    validate_entry_path(path).map_err(|_| ManifestError::InvalidIcon)
}

pub fn validate_permission_value(value: &str) -> Result<(), ManifestError> {
    if value.is_empty() || value.len() > MAX_PERMISSION_VALUE_LEN {
        return Err(ManifestError::InvalidFormat);
    }
    if !value
        .bytes()
        .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
    {
        return Err(ManifestError::InvalidFormat);
    }
    Ok(())
}

fn is_ascii_lower_or_digit(byte: u8) -> bool {
    matches!(byte, b'a'..=b'z' | b'0'..=b'9')
}

fn set_icon_bit(pixels: &mut [u8; PACKAGE_ICON_1BPP_BYTES], x: usize, y: usize) {
    let bit = y * PACKAGE_ICON_WIDTH + x;
    pixels[bit / 8] |= 0x80 >> (bit % 8);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageList {
    items: [Option<PackageInfo>; MAX_PACKAGES],
    len: usize,
}

impl PackageList {
    pub const fn new() -> Self {
        Self {
            items: [None; MAX_PACKAGES],
            len: 0,
        }
    }

    pub fn push(&mut self, item: PackageInfo) -> bool {
        if self.len >= MAX_PACKAGES {
            return false;
        }
        self.items[self.len] = Some(item);
        self.len += 1;
        true
    }

    /// Empty the list in place, slot by slot. Deliberately not
    /// `*self = PackageList::new()`: assigning a fresh ~28 KiB value can
    /// materialize it on the caller's stack first, which is exactly what the
    /// device shell's in-place reload exists to avoid (KOTO-0172).
    pub fn clear(&mut self) {
        for slot in &mut self.items {
            *slot = None;
        }
        self.len = 0;
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> Option<&PackageInfo> {
        if index >= self.len {
            return None;
        }
        self.items[index].as_ref()
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut PackageInfo> {
        if index >= self.len {
            return None;
        }
        self.items[index].as_mut()
    }

    pub fn iter(&self) -> PackageIter<'_> {
        PackageIter {
            list: self,
            index: 0,
        }
    }
}

impl Default for PackageList {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PackageIter<'a> {
    list: &'a PackageList,
    index: usize,
}

impl<'a> Iterator for PackageIter<'a> {
    type Item = &'a PackageInfo;

    fn next(&mut self) -> Option<Self::Item> {
        let item = self.list.get(self.index);
        self.index += 1;
        item
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_info_rejects_empty_fields() {
        assert!(PackageInfo::new("", "Name").is_none());
        assert!(PackageInfo::new("dev.koto.app", "").is_none());
    }

    #[test]
    fn parses_text_package_icon() {
        let mut text = std::string::String::from("KICON1\n");
        for y in 0..PACKAGE_ICON_HEIGHT {
            for x in 0..PACKAGE_ICON_WIDTH {
                text.push(if x == y { '#' } else { '.' });
            }
            text.push('\n');
        }

        let icon = PackageIcon::from_kicon_text(text.as_bytes()).unwrap();

        assert!(icon.pixel(0, 0));
        assert!(icon.pixel(12, 12));
        assert!(!icon.pixel(12, 13));
    }

    #[test]
    fn manifest_accepts_required_fields() {
        let manifest = PackageManifest::new(ManifestFields {
            format: KPA_MANIFEST_FORMAT,
            version: KPA_MANIFEST_VERSION,
            app_id: "dev.koto.sample",
            name: "Sample App",
            runtime: "kotoruntime-bytecode",
            entry: "bytecode/main.kbc",
            icon: Some("icons/sample.kicon"),
            shell_icon: None,
            fs_permission: Some("sandbox"),
            network_permission: Some(false),
            sram_work_bytes: Some(16_384),
            psram_cache_bytes: Some(65_536),
            description: Some("A sample app."),
            category: Some("Tools"),
        })
        .unwrap();

        assert_eq!(manifest.package().app_id(), "dev.koto.sample");
        assert_eq!(manifest.package().description(), Some("A sample app."));
        assert_eq!(manifest.package().category(), Some("Tools"));
        assert_eq!(manifest.package().name(), "Sample App");
        assert_eq!(manifest.package().icon_path(), Some("icons/sample.kicon"));
        assert_eq!(manifest.runtime(), "kotoruntime-bytecode");
        assert_eq!(manifest.entry(), "bytecode/main.kbc");
        assert_eq!(manifest.package().runtime(), Some("kotoruntime-bytecode"));
        assert_eq!(manifest.package().entry(), Some("bytecode/main.kbc"));
        assert_eq!(manifest.package().fs_permission(), Some("sandbox"));
        assert_eq!(manifest.package().network_permission(), Some(false));
        assert_eq!(manifest.package().sram_work_bytes(), Some(16_384));
        assert_eq!(manifest.package().psram_cache_bytes(), Some(65_536));
    }

    #[test]
    fn manifest_rejects_invalid_identifiers() {
        let valid = ManifestFields {
            format: KPA_MANIFEST_FORMAT,
            version: KPA_MANIFEST_VERSION,
            app_id: "dev.koto.sample",
            name: "Sample App",
            runtime: "kotoruntime-bytecode",
            entry: "bytecode/main.kbc",
            icon: None,
            shell_icon: None,
            fs_permission: None,
            network_permission: None,
            sram_work_bytes: None,
            psram_cache_bytes: None,
            description: None,
            category: None,
        };

        assert_eq!(
            PackageManifest::new(ManifestFields {
                app_id: "Dev.Koto.Sample",
                ..valid
            }),
            Err(ManifestError::InvalidAppId)
        );
        assert_eq!(
            PackageManifest::new(ManifestFields {
                runtime: "KotoRuntime",
                ..valid
            }),
            Err(ManifestError::InvalidRuntime)
        );
        assert_eq!(
            PackageManifest::new(ManifestFields {
                entry: "../main.kbc",
                ..valid
            }),
            Err(ManifestError::InvalidEntry)
        );
        assert_eq!(
            PackageManifest::new(ManifestFields {
                icon: Some("../icon.kicon"),
                ..valid
            }),
            Err(ManifestError::InvalidIcon)
        );
    }

    #[test]
    fn manifest_description_and_category_are_optional() {
        let manifest = PackageManifest::new(ManifestFields {
            format: KPA_MANIFEST_FORMAT,
            version: KPA_MANIFEST_VERSION,
            app_id: "dev.koto.sample",
            name: "Sample App",
            runtime: "kotoruntime-bytecode",
            entry: "bytecode/main.kbc",
            icon: None,
            shell_icon: None,
            fs_permission: None,
            network_permission: None,
            sram_work_bytes: None,
            psram_cache_bytes: None,
            description: None,
            category: None,
        })
        .unwrap();

        assert_eq!(manifest.package().description(), None);
        assert_eq!(manifest.package().category(), None);
    }

    #[test]
    fn manifest_rejects_overlong_description_and_category() {
        let base = ManifestFields {
            format: KPA_MANIFEST_FORMAT,
            version: KPA_MANIFEST_VERSION,
            app_id: "dev.koto.sample",
            name: "Sample App",
            runtime: "kotoruntime-bytecode",
            entry: "bytecode/main.kbc",
            icon: None,
            shell_icon: None,
            fs_permission: None,
            network_permission: None,
            sram_work_bytes: None,
            psram_cache_bytes: None,
            description: None,
            category: None,
        };

        let long_description = "x".repeat(MAX_DESCRIPTION_LEN + 1);
        assert_eq!(
            PackageManifest::new(ManifestFields {
                description: Some(&long_description),
                ..base
            }),
            Err(ManifestError::InvalidDescription)
        );

        let long_category = "y".repeat(MAX_CATEGORY_LEN + 1);
        assert_eq!(
            PackageManifest::new(ManifestFields {
                category: Some(&long_category),
                ..base
            }),
            Err(ManifestError::InvalidCategory)
        );
    }

    #[test]
    fn package_list_keeps_insertion_order() {
        let mut list = PackageList::new();
        assert!(list.push(PackageInfo::new("dev.koto.one", "One").unwrap()));
        assert!(list.push(PackageInfo::new("dev.koto.two", "Two").unwrap()));

        let names: std::vec::Vec<&str> = list.iter().map(PackageInfo::name).collect();
        assert_eq!(names, ["One", "Two"]);
    }
}
