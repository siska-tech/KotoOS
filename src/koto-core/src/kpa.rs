use crate::package::validate_entry_path;

pub const KPA_MAGIC: &[u8; 4] = b"KPA1";
pub const KPA_VERSION_MAJOR: u16 = 1;
pub const KPA_VERSION_MINOR: u16 = 0;
pub const KPA_HEADER_SIZE: usize = 64;
pub const KPA_ENTRY_SIZE: usize = 64;
pub const KPA_FIRST_ASSET_ALIGNMENT: u32 = 4096;
pub const KPA_PAYLOAD_ALIGNMENT: u32 = 512;
pub const KPA_FLAG_SEQUENTIAL: u32 = 1 << 0;
pub const KPA_FLAG_PRELOAD: u32 = 1 << 1;
pub const KPA_FLAG_ENTRY: u32 = 1 << 2;
const KPA_KNOWN_FLAGS: u32 = KPA_FLAG_SEQUENTIAL | KPA_FLAG_PRELOAD | KPA_FLAG_ENTRY;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KpaError {
    TooSmall,
    InvalidMagic,
    UnsupportedVersion,
    InvalidHeader,
    InvalidReserved,
    InvalidRange,
    InvalidEntry,
    InvalidPath,
    InvalidFlags,
    NonMonotonicAsset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KpaHeader {
    pub entry_count: u32,
    pub table_offset: u32,
    pub string_table_offset: u32,
    pub string_table_size: u32,
    pub metadata_offset: u32,
    pub metadata_size: u32,
    pub first_asset_offset: u32,
    pub package_size: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KpaEntry<'a> {
    pub path: &'a str,
    pub asset_type: u32,
    pub flags: u32,
    pub data_offset: u32,
    pub data_size: u32,
    pub alignment: u32,
}

impl KpaEntry<'_> {
    pub fn is_sequential(&self) -> bool {
        self.flags & KPA_FLAG_SEQUENTIAL != 0
    }

    pub fn wants_preload(&self) -> bool {
        self.flags & KPA_FLAG_PRELOAD != 0
    }

    pub fn is_entry(&self) -> bool {
        self.flags & KPA_FLAG_ENTRY != 0
    }

    pub fn payload_window(&self) -> PreloadWindow {
        PreloadWindow {
            offset: self.data_offset,
            size: self.data_size,
        }
    }

    pub fn preload_window(&self, max_bytes: u32) -> Option<PreloadWindow> {
        if !self.wants_preload() {
            return None;
        }
        Some(PreloadWindow {
            offset: self.data_offset,
            size: self.data_size.min(max_bytes),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PreloadWindow {
    pub offset: u32,
    pub size: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KpaReader<'a> {
    bytes: &'a [u8],
    header: KpaHeader,
}

impl<'a> KpaReader<'a> {
    pub fn new(bytes: &'a [u8]) -> Result<Self, KpaError> {
        let header = parse_header(bytes)?;
        validate_ranges(bytes, header)?;

        let reader = Self { bytes, header };
        let mut previous_end = header.first_asset_offset;
        for index in 0..header.entry_count {
            let entry = reader.entry(index)?;
            if entry.data_offset < previous_end {
                return Err(KpaError::NonMonotonicAsset);
            }
            previous_end = entry
                .data_offset
                .checked_add(entry.data_size)
                .ok_or(KpaError::InvalidRange)?;
        }

        Ok(reader)
    }

    pub fn header(&self) -> KpaHeader {
        self.header
    }

    pub fn entry_count(&self) -> u32 {
        self.header.entry_count
    }

    pub fn entry(&self, index: u32) -> Result<KpaEntry<'a>, KpaError> {
        if index >= self.header.entry_count {
            return Err(KpaError::InvalidEntry);
        }

        let offset = checked_add(
            self.header.table_offset,
            index
                .checked_mul(KPA_ENTRY_SIZE as u32)
                .ok_or(KpaError::InvalidRange)?,
        )?;
        let record = read_range(self.bytes, offset, KPA_ENTRY_SIZE)?;
        let path_offset = read_u32(record, 0);
        let path_len = read_u32(record, 4);
        let asset_type = read_u32(record, 8);
        let flags = read_u32(record, 12);
        let data_offset = read_u32(record, 16);
        let data_size = read_u32(record, 20);
        let alignment = read_u32(record, 24);
        let reserved0 = read_u32(record, 28);

        if flags & !KPA_KNOWN_FLAGS != 0 || reserved0 != 0 || record[32..64].iter().any(|b| *b != 0)
        {
            return Err(KpaError::InvalidFlags);
        }
        if alignment == 0 || !data_offset.is_multiple_of(alignment) {
            return Err(KpaError::InvalidEntry);
        }
        let data_end = checked_add(data_offset, data_size)?;
        if data_offset < self.header.first_asset_offset || data_end > self.header.package_size {
            return Err(KpaError::InvalidRange);
        }

        let path_start = checked_add(self.header.string_table_offset, path_offset)?;
        let path_bytes = read_range(self.bytes, path_start, path_len as usize)?;
        let path = core::str::from_utf8(path_bytes).map_err(|_| KpaError::InvalidPath)?;
        validate_entry_path(path).map_err(|_| KpaError::InvalidPath)?;

        Ok(KpaEntry {
            path,
            asset_type,
            flags,
            data_offset,
            data_size,
            alignment,
        })
    }

    pub fn find_entry(&self, path: &str) -> Result<Option<KpaEntry<'a>>, KpaError> {
        for index in 0..self.header.entry_count {
            let entry = self.entry(index)?;
            if entry.path == path {
                return Ok(Some(entry));
            }
        }
        Ok(None)
    }

    pub fn preload_window_for(
        &self,
        path: &str,
        max_bytes: u32,
    ) -> Result<Option<PreloadWindow>, KpaError> {
        Ok(self
            .find_entry(path)?
            .and_then(|entry| entry.preload_window(max_bytes)))
    }
}

fn parse_header(bytes: &[u8]) -> Result<KpaHeader, KpaError> {
    if bytes.len() < KPA_HEADER_SIZE {
        return Err(KpaError::TooSmall);
    }
    if &bytes[..4] != KPA_MAGIC {
        return Err(KpaError::InvalidMagic);
    }
    if read_u16(bytes, 4) != KPA_VERSION_MAJOR || read_u16(bytes, 6) != KPA_VERSION_MINOR {
        return Err(KpaError::UnsupportedVersion);
    }
    if read_u32(bytes, 8) as usize != KPA_HEADER_SIZE || read_u32(bytes, 12) != 0 {
        return Err(KpaError::InvalidHeader);
    }
    if bytes[48..64].iter().any(|b| *b != 0) {
        return Err(KpaError::InvalidReserved);
    }

    Ok(KpaHeader {
        entry_count: read_u32(bytes, 16),
        table_offset: read_u32(bytes, 20),
        string_table_offset: read_u32(bytes, 24),
        string_table_size: read_u32(bytes, 28),
        metadata_offset: read_u32(bytes, 32),
        metadata_size: read_u32(bytes, 36),
        first_asset_offset: read_u32(bytes, 40),
        package_size: read_u32(bytes, 44),
    })
}

fn validate_ranges(bytes: &[u8], header: KpaHeader) -> Result<(), KpaError> {
    if header.table_offset as usize != KPA_HEADER_SIZE {
        return Err(KpaError::InvalidHeader);
    }
    if !header
        .first_asset_offset
        .is_multiple_of(KPA_FIRST_ASSET_ALIGNMENT)
    {
        return Err(KpaError::InvalidHeader);
    }
    if header.package_size as usize != bytes.len() {
        return Err(KpaError::InvalidRange);
    }

    let table_size = header
        .entry_count
        .checked_mul(KPA_ENTRY_SIZE as u32)
        .ok_or(KpaError::InvalidRange)?;
    let table_end = checked_add(header.table_offset, table_size)?;
    let string_end = checked_add(header.string_table_offset, header.string_table_size)?;
    let metadata_end = checked_add(header.metadata_offset, header.metadata_size)?;

    if table_end > header.string_table_offset
        || string_end > header.metadata_offset
        || metadata_end > header.first_asset_offset
        || header.first_asset_offset > header.package_size
    {
        return Err(KpaError::InvalidRange);
    }

    read_range(bytes, header.table_offset, table_size as usize)?;
    read_range(
        bytes,
        header.string_table_offset,
        header.string_table_size as usize,
    )?;
    read_range(bytes, header.metadata_offset, header.metadata_size as usize)?;
    Ok(())
}

fn read_range(bytes: &[u8], offset: u32, size: usize) -> Result<&[u8], KpaError> {
    let offset = offset as usize;
    let end = offset.checked_add(size).ok_or(KpaError::InvalidRange)?;
    bytes.get(offset..end).ok_or(KpaError::InvalidRange)
}

fn checked_add(left: u32, right: u32) -> Result<u32, KpaError> {
    left.checked_add(right).ok_or(KpaError::InvalidRange)
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u16(bytes: &mut Vec<u8>, value: u16) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn package_with_offsets(first_offset: u32, second_offset: u32) -> Vec<u8> {
        let first_path = b"bytecode/main.kbc";
        let second_path = b"assets/title.rle";
        let strings_size = (first_path.len() + second_path.len()) as u32;
        let metadata = b"{}";
        let metadata_offset = 64 + 64 * 2 + strings_size;
        let first_asset_offset = 4096;
        let package_size = first_offset.max(second_offset) + 512;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(KPA_MAGIC);
        push_u16(&mut bytes, 1);
        push_u16(&mut bytes, 0);
        push_u32(&mut bytes, 64);
        push_u32(&mut bytes, 0);
        push_u32(&mut bytes, 2);
        push_u32(&mut bytes, 64);
        push_u32(&mut bytes, 64 + 64 * 2);
        push_u32(&mut bytes, strings_size);
        push_u32(&mut bytes, metadata_offset);
        push_u32(&mut bytes, metadata.len() as u32);
        push_u32(&mut bytes, first_asset_offset);
        push_u32(&mut bytes, package_size);
        bytes.extend_from_slice(&[0; 16]);

        write_entry(
            &mut bytes,
            0,
            first_path.len() as u32,
            KPA_FLAG_ENTRY,
            first_offset,
            20,
        );
        write_entry(
            &mut bytes,
            first_path.len() as u32,
            second_path.len() as u32,
            KPA_FLAG_PRELOAD,
            second_offset,
            10,
        );
        bytes.extend_from_slice(first_path);
        bytes.extend_from_slice(second_path);
        bytes.extend_from_slice(metadata);
        bytes.resize(package_size as usize, 0);
        bytes
    }

    fn write_entry(
        bytes: &mut Vec<u8>,
        path_offset: u32,
        path_len: u32,
        flags: u32,
        data_offset: u32,
        data_size: u32,
    ) {
        push_u32(bytes, path_offset);
        push_u32(bytes, path_len);
        push_u32(bytes, 1);
        push_u32(bytes, flags);
        push_u32(bytes, data_offset);
        push_u32(bytes, data_size);
        push_u32(bytes, KPA_PAYLOAD_ALIGNMENT);
        push_u32(bytes, 0);
        bytes.extend_from_slice(&[0; 32]);
    }

    #[test]
    fn parses_entries_and_preload_windows() {
        let bytes = package_with_offsets(4096, 4608);
        let reader = KpaReader::new(&bytes).unwrap();

        let entry = reader.entry(0).unwrap();
        assert_eq!(entry.path, "bytecode/main.kbc");
        assert!(entry.is_entry());

        assert_eq!(
            reader.preload_window_for("assets/title.rle", 4).unwrap(),
            Some(PreloadWindow {
                offset: 4608,
                size: 4
            })
        );
        assert_eq!(
            reader.preload_window_for("bytecode/main.kbc", 4).unwrap(),
            None
        );
    }

    #[test]
    fn rejects_non_monotonic_asset_offsets() {
        let bytes = package_with_offsets(4608, 4096);

        assert_eq!(KpaReader::new(&bytes), Err(KpaError::NonMonotonicAsset));
    }
}
