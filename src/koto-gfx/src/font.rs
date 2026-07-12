//! Read-only access to the compact `.kfont` bitmap font blob.
//!
//! The blob is produced offline from M+ BITMAP FONTS by
//! `harness/mplus_to_kfont.py`; this module only interprets the already-clean,
//! fixed-cell binary. It is `no_std` and never allocates: a [`BitmapFont`]
//! borrows the blob bytes and [`glyph`](BitmapFont::glyph) returns a borrowed
//! [`Glyph`] view. Missing glyphs report `None` rather than panicking.
//!
//! Layout (little-endian) is documented in `harness/mplus_to_kfont.py`.

const MAGIC: &[u8; 4] = b"KFNT";
const VERSION: u16 = 1;
const HEADER_LEN: usize = 16;
const INDEX_ENTRY_LEN: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FontError {
    BadMagic,
    UnsupportedVersion,
    Truncated,
}

/// A borrowed view over the `.kfont` blob.
#[derive(Clone, Copy, Debug)]
pub struct BitmapFont<'a> {
    data: &'a [u8],
    cell_h: u8,
    ascent: u8,
    half_w: u8,
    full_w: u8,
    glyph_count: u32,
    index_off: usize,
    bitmap_off: usize,
}

/// A single fixed-cell glyph: `width` columns by `font.cell_height()` rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Glyph<'a> {
    width: u8,
    height: u8,
    row_bytes: u8,
    bits: &'a [u8],
}

impl<'a> BitmapFont<'a> {
    pub fn from_bytes(data: &'a [u8]) -> Result<Self, FontError> {
        if data.len() < HEADER_LEN {
            return Err(FontError::Truncated);
        }
        if &data[0..4] != MAGIC {
            return Err(FontError::BadMagic);
        }
        let version = read_u16(data, 4);
        if version != VERSION {
            return Err(FontError::UnsupportedVersion);
        }
        let cell_h = data[8];
        let ascent = data[9];
        let half_w = data[10];
        let full_w = data[11];
        let glyph_count = read_u32(data, 12);

        let index_off = HEADER_LEN;
        let index_len = (glyph_count as usize)
            .checked_mul(INDEX_ENTRY_LEN)
            .ok_or(FontError::Truncated)?;
        let bitmap_off = index_off
            .checked_add(index_len)
            .ok_or(FontError::Truncated)?;
        if bitmap_off > data.len() {
            return Err(FontError::Truncated);
        }

        Ok(Self {
            data,
            cell_h,
            ascent,
            half_w,
            full_w,
            glyph_count,
            index_off,
            bitmap_off,
        })
    }

    pub fn cell_height(&self) -> u8 {
        self.cell_h
    }

    pub fn ascent(&self) -> u8 {
        self.ascent
    }

    /// Advance width of half-width (Latin) glyphs.
    pub fn half_width(&self) -> u8 {
        self.half_w
    }

    /// Advance width of full-width (CJK) glyphs.
    pub fn full_width(&self) -> u8 {
        self.full_w
    }

    pub fn glyph_count(&self) -> u32 {
        self.glyph_count
    }

    /// Look up a glyph by Unicode codepoint, or `None` if it is not stored.
    pub fn glyph(&self, ch: char) -> Option<Glyph<'a>> {
        let target = ch as u32;
        let count = self.glyph_count as usize;
        let (mut lo, mut hi) = (0usize, count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let cp = self.index_codepoint(mid);
            if cp == target {
                return self.glyph_at(mid);
            } else if cp < target {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        None
    }

    fn index_codepoint(&self, i: usize) -> u32 {
        read_u32(self.data, self.index_off + i * INDEX_ENTRY_LEN)
    }

    fn glyph_at(&self, i: usize) -> Option<Glyph<'a>> {
        let base = self.index_off + i * INDEX_ENTRY_LEN;
        let width = self.data[base + 4];
        let row_bytes = self.data[base + 5];
        let off = read_u32(self.data, base + 6) as usize;
        let len = row_bytes as usize * self.cell_h as usize;
        let start = self.bitmap_off + off;
        let bits = self.data.get(start..start + len)?;
        Some(Glyph {
            width,
            height: self.cell_h,
            row_bytes,
            bits,
        })
    }
}

impl<'a> Glyph<'a> {
    pub fn width(&self) -> u8 {
        self.width
    }

    pub fn height(&self) -> u8 {
        self.height
    }

    /// Returns true if the pixel at (`x`, `y`) is set. Out-of-bounds is false.
    pub fn pixel(&self, x: u8, y: u8) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let byte_index = y as usize * self.row_bytes as usize + (x / 8) as usize;
        match self.bits.get(byte_index) {
            Some(byte) => (byte >> (7 - (x % 8))) & 1 == 1,
            None => false,
        }
    }
}

fn read_u16(data: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([data[off], data[off + 1]])
}

fn read_u32(data: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a tiny in-memory font with two glyphs for codepoints 'A' and '\u{3042}'.
    fn sample_font() -> std::vec::Vec<u8> {
        let cell_h = 2u8;
        // 'A' (0x41): half width 6, row_bytes 1, rows 0b10000000, 0b01000000
        // 'あ' (0x3042): full width 12, row_bytes 2
        let mut data = std::vec::Vec::new();
        data.extend_from_slice(MAGIC);
        data.extend_from_slice(&VERSION.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes()); // flags
        data.push(cell_h);
        data.push(1); // ascent (unused in test)
        data.push(6); // half_w
        data.push(12); // full_w
        data.extend_from_slice(&2u32.to_le_bytes()); // glyph_count

        // index entries must be sorted ascending by codepoint
        // 'A' bitmap at offset 0 (2 bytes), 'あ' at offset 2 (4 bytes)
        data.extend_from_slice(&0x41u32.to_le_bytes());
        data.extend_from_slice(&[6, 1]);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0x3042u32.to_le_bytes());
        data.extend_from_slice(&[12, 2]);
        data.extend_from_slice(&2u32.to_le_bytes());

        // bitmap blob
        data.extend_from_slice(&[0b1000_0000, 0b0100_0000]); // 'A'
        data.extend_from_slice(&[0b1000_0000, 0b0000_0000, 0b1000_0000, 0b0001_0000]); // 'あ'
        data
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        assert_eq!(
            BitmapFont::from_bytes(b"XXXX............").err(),
            Some(FontError::BadMagic)
        );
        let mut bytes = sample_font();
        bytes[4] = 99; // version
        assert_eq!(
            BitmapFont::from_bytes(&bytes).err(),
            Some(FontError::UnsupportedVersion)
        );
    }

    #[test]
    fn reads_metrics() {
        let bytes = sample_font();
        let font = BitmapFont::from_bytes(&bytes).unwrap();
        assert_eq!(font.cell_height(), 2);
        assert_eq!(font.half_width(), 6);
        assert_eq!(font.full_width(), 12);
        assert_eq!(font.glyph_count(), 2);
    }

    #[test]
    fn looks_up_ascii_and_japanese_glyphs() {
        let bytes = sample_font();
        let font = BitmapFont::from_bytes(&bytes).unwrap();

        let a = font.glyph('A').expect("ascii glyph");
        assert_eq!(a.width(), 6);
        assert!(a.pixel(0, 0));
        assert!(!a.pixel(1, 0));
        assert!(a.pixel(1, 1));

        let kana = font.glyph('\u{3042}').expect("japanese glyph");
        assert_eq!(kana.width(), 12);
        assert!(kana.pixel(0, 0));
        assert!(kana.pixel(11, 1)); // last column of second row (0b0000_0001)
    }

    #[test]
    fn missing_glyph_returns_none_without_panicking() {
        let bytes = sample_font();
        let font = BitmapFont::from_bytes(&bytes).unwrap();
        assert!(font.glyph('Z').is_none());
        assert!(font.glyph('\u{5B57}').is_none());
    }
}
