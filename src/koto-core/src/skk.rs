//! SKK dictionary lookup with an SRAM-resident leading-character index.
//!
//! Full SKK dictionaries are megabytes of sorted text and live on the SD card,
//! which is SPI-only with expensive random access (HC-6). Loading the whole file
//! into the RP2040's 264KB SRAM is impossible, so this module keeps only a small
//! index in SRAM: one `(leading character -> byte offset)` entry per distinct
//! reading-initial character. A lookup binary-searches that index for the
//! target's leading character, seeks once to the bucket start, then scans
//! forward sequentially until the exact reading is found or passed. This costs at
//! most one random seek per conversion and keeps reads sequential afterwards.
//!
//! The dictionary can be accessed two ways with the same index and parsing
//! code: as a borrowed byte slice (in-memory fixtures, host tests) or through
//! a [`SkkRead`] windowed reader that fetches at most [`SKK_LOOKUP_WINDOW_BYTES`]
//! at a time, which is how hardware reads a multi-KB dictionary straight off
//! the SD card without holding it in SRAM. See
//! [docs/spec/SKK_DICTIONARY.md](../../../docs/spec/SKK_DICTIONARY.md) for the file format
//! assumptions and memory budget.

/// Maximum UTF-8 bytes in one leading-character index key (one scalar value).
pub const MAX_LEADING_KEY_BYTES: usize = 4;

/// Default index capacity: enough distinct leading characters for hiragana,
/// katakana, and ASCII readings in a realistic SKK dictionary.
pub const DEFAULT_INDEX_CAPACITY: usize = 192;

/// Scan-window byte budget for [`SkkIndex::lookup_in_reader`] /
/// [`SkkIndex::build_from_reader`]. This is a packaging contract: every
/// dictionary line (with newline) must fit the window, or windowed lookups
/// skip the overlong line. The shipped `skk_koto.skk`'s longest line is 168
/// bytes, so 512 leaves ~3x headroom; the contract is guarded by
/// `koto_dict_lines_fit_the_lookup_window`.
pub const SKK_LOOKUP_WINDOW_BYTES: usize = 512;

/// Convenience alias for an index with the default capacity.
pub type SkkLeadingIndex = SkkIndex<DEFAULT_INDEX_CAPACITY>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkkError {
    /// The dictionary has more distinct leading characters than the index can hold.
    IndexFull,
    /// The dictionary is larger than the 32-bit offset space the index uses.
    DictionaryTooLarge,
    /// The backing storage failed mid-read (windowed access only).
    Io,
}

/// Random-access byte source for a dictionary that is not memory-resident.
///
/// Implementations may return short reads; callers loop. `Ok(0)` means the
/// offset is at or past the end of the dictionary.
pub trait SkkRead {
    /// Read up to `buf.len()` bytes starting at absolute byte `offset`.
    fn read_at(&mut self, offset: u32, buf: &mut [u8]) -> Result<usize, SkkError>;
}

/// In-memory dictionaries are readers too, so tests and the sim exercise the
/// exact windowed code path the hardware uses.
impl SkkRead for &[u8] {
    fn read_at(&mut self, offset: u32, buf: &mut [u8]) -> Result<usize, SkkError> {
        let start = (offset as usize).min(self.len());
        let n = buf.len().min(self.len() - start);
        buf[..n].copy_from_slice(&self[start..start + n]);
        Ok(n)
    }
}

/// Fill `buf` from `offset`, looping over short reads; returns bytes read
/// (less than `buf.len()` only at end of dictionary).
fn read_fully<R: SkkRead>(reader: &mut R, offset: u32, buf: &mut [u8]) -> Result<usize, SkkError> {
    let mut got = 0;
    while got < buf.len() {
        match reader.read_at(offset + got as u32, &mut buf[got..])? {
            0 => break,
            n => got += n,
        }
    }
    Ok(got)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct IndexEntry {
    key: [u8; MAX_LEADING_KEY_BYTES],
    key_len: u8,
    offset: u32,
}

impl IndexEntry {
    const EMPTY: IndexEntry = IndexEntry {
        key: [0; MAX_LEADING_KEY_BYTES],
        key_len: 0,
        offset: 0,
    };

    fn key_slice(&self) -> &[u8] {
        &self.key[..self.key_len as usize]
    }
}

/// SRAM-resident leading-character index over a sorted SKK dictionary.
///
/// `N` is the maximum number of distinct leading characters. Each entry is a few
/// bytes, so even a full hiragana/katakana set stays well under 2KB.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkkIndex<const N: usize> {
    entries: [IndexEntry; N],
    count: usize,
    dict_len: u32,
}

impl<const N: usize> SkkIndex<N> {
    /// Build the index by scanning the dictionary once, sequentially.
    ///
    /// The dictionary must be UTF-8 and sorted ascending by reading. Comment
    /// lines (starting with `;`), blank lines, and malformed lines are skipped.
    pub fn build(dict: &[u8]) -> Result<Self, SkkError> {
        if dict.len() > u32::MAX as usize {
            return Err(SkkError::DictionaryTooLarge);
        }

        let mut entries = [IndexEntry::EMPTY; N];
        let mut count = 0;
        let mut offset = 0;

        while offset < dict.len() {
            let (line, next) = line_at(dict, offset);
            if let Some(entry) = parse_entry(line) {
                Self::note_reading(entry.reading, offset as u32, &mut entries, &mut count)?;
            }
            offset = next;
        }

        Ok(Self {
            entries,
            count,
            dict_len: dict.len() as u32,
        })
    }

    /// Build the index by streaming the dictionary once, sequentially, through
    /// `reader` in `window`-sized chunks — the boot path on hardware, where the
    /// dictionary never becomes SRAM-resident.
    ///
    /// Semantics match [`SkkIndex::build`]: comments, blanks, and malformed
    /// lines are skipped. A line longer than the window still registers its
    /// bucket from the visible head (the reading and its ` /` separator always
    /// fit any sane window) and is then skipped through.
    pub fn build_from_reader<R: SkkRead>(
        reader: &mut R,
        window: &mut [u8],
    ) -> Result<Self, SkkError> {
        let mut index = Self::empty();
        Self::build_from_reader_into(reader, window, &mut index)?;
        Ok(index)
    }

    /// Empty index: no buckets over a zero-length dictionary.
    pub const fn empty() -> Self {
        Self {
            entries: [IndexEntry::EMPTY; N],
            count: 0,
            dict_len: 0,
        }
    }

    /// [`Self::build_from_reader`] writing through `out` instead of returning
    /// the index by value (KOTO-0252): on the device the ~2.3 KiB index would
    /// otherwise land on the launch-path frame as a temporary before reaching
    /// its resident cell. On `Err`, `out` holds an unspecified partial index
    /// and must not be used.
    pub fn build_from_reader_into<R: SkkRead>(
        reader: &mut R,
        window: &mut [u8],
        out: &mut Self,
    ) -> Result<(), SkkError> {
        out.entries.fill(IndexEntry::EMPTY);
        out.count = 0;
        out.dict_len = 0;
        let entries = &mut out.entries;
        let mut count = 0;
        let mut offset: u64 = 0;

        loop {
            if offset > u32::MAX as u64 {
                return Err(SkkError::DictionaryTooLarge);
            }
            let n = read_fully(reader, offset as u32, window)?;
            if n == 0 {
                break;
            }
            let chunk = &window[..n];
            let mut pos = 0;
            while let Some(i) = chunk[pos..].iter().position(|&b| b == b'\n') {
                if let Some(entry) = parse_entry(&chunk[pos..pos + i]) {
                    Self::note_reading(
                        entry.reading,
                        offset as u32 + pos as u32,
                        entries,
                        &mut count,
                    )?;
                }
                pos += i + 1;
            }
            if pos > 0 {
                // Complete lines consumed; the partial tail line (if any) is
                // re-read from its own start next iteration.
                offset += pos as u64;
                continue;
            }
            if n < window.len() {
                // `read_fully` came up short with no newline: final line at EOF.
                if let Some(entry) = parse_entry(chunk) {
                    Self::note_reading(entry.reading, offset as u32, entries, &mut count)?;
                }
                offset += n as u64;
                break;
            }
            // A full window without a newline: the line overflows the window
            // (out of the packaging contract). Register its bucket from the
            // head, then stream forward to the next newline.
            if let Some(reading) = parse_reading_prefix(chunk) {
                Self::note_reading(reading, offset as u32, entries, &mut count)?;
            }
            offset += n as u64;
            loop {
                if offset > u32::MAX as u64 {
                    return Err(SkkError::DictionaryTooLarge);
                }
                let n = reader.read_at(offset as u32, window)?;
                if n == 0 {
                    break;
                }
                match window[..n].iter().position(|&b| b == b'\n') {
                    Some(i) => {
                        offset += i as u64 + 1;
                        break;
                    }
                    None => offset += n as u64,
                }
            }
        }

        if offset > u32::MAX as u64 {
            return Err(SkkError::DictionaryTooLarge);
        }
        out.count = count;
        out.dict_len = offset as u32;
        Ok(())
    }

    /// Register `reading`'s leading character as a bucket if it starts one.
    /// The dictionary is sorted, so equal leading characters are contiguous
    /// and only a change of key opens a new bucket.
    fn note_reading(
        reading: &str,
        line_offset: u32,
        entries: &mut [IndexEntry; N],
        count: &mut usize,
    ) -> Result<(), SkkError> {
        let Some(c) = reading.chars().next() else {
            return Ok(());
        };
        let key_bytes = &reading.as_bytes()[..c.len_utf8()];
        let is_new_bucket = *count == 0 || entries[*count - 1].key_slice() != key_bytes;
        if is_new_bucket {
            if *count == N {
                return Err(SkkError::IndexFull);
            }
            let mut key = [0u8; MAX_LEADING_KEY_BYTES];
            key[..key_bytes.len()].copy_from_slice(key_bytes);
            entries[*count] = IndexEntry {
                key,
                key_len: key_bytes.len() as u8,
                offset: line_offset,
            };
            *count += 1;
        }
        Ok(())
    }

    /// Number of leading-character buckets in the index.
    pub fn entry_count(&self) -> usize {
        self.count
    }

    /// Approximate SRAM footprint of the live index entries, in bytes.
    pub fn sram_bytes(&self) -> usize {
        self.count * core::mem::size_of::<IndexEntry>()
    }

    /// Look up the exact `reading` and return its dictionary entry, if present.
    ///
    /// `dict` must be the same byte slice the index was built from. The returned
    /// [`DictEntry`] borrows from it.
    pub fn lookup<'a>(&self, dict: &'a [u8], reading: &str) -> Option<DictEntry<'a>> {
        let first = reading.chars().next()?;
        let key = &reading.as_bytes()[..first.len_utf8()];
        let bucket = self.find_bucket(key)?;

        let start = self.entries[bucket].offset as usize;
        let end = if bucket + 1 < self.count {
            self.entries[bucket + 1].offset as usize
        } else {
            self.dict_len as usize
        };

        let mut offset = start;
        while offset < end {
            let (line, next) = line_at(dict, offset);
            if let Some(entry) = parse_entry(line) {
                match entry.reading.cmp(reading) {
                    core::cmp::Ordering::Less => {}
                    core::cmp::Ordering::Equal => return Some(entry),
                    // The dictionary is sorted, so once we pass the reading it is absent.
                    core::cmp::Ordering::Greater => return None,
                }
            }
            offset = next;
        }
        None
    }

    /// Total dictionary length in bytes, as recorded at index build time.
    pub fn dict_len(&self) -> u32 {
        self.dict_len
    }

    /// Look up `reading` through a windowed reader: one bucket seek, then a
    /// forward scan fetching at most `window.len()` bytes at a time. On a hit
    /// the matched line is re-read into `window` and the returned entry
    /// borrows it. This is the hardware path (HC-6): the SD card pays one
    /// random seek per conversion and sequential reads afterwards.
    ///
    /// Lines longer than the window can never be returned; they are skipped,
    /// matching the malformed-line policy. The packaging contract
    /// ([`SKK_LOOKUP_WINDOW_BYTES`]) keeps real dictionaries far below that.
    pub fn lookup_in_reader<'w, R: SkkRead>(
        &self,
        reader: &mut R,
        reading: &str,
        window: &'w mut [u8],
    ) -> Result<Option<DictEntry<'w>>, SkkError> {
        let Some(first) = reading.chars().next() else {
            return Ok(None);
        };
        let key = &reading.as_bytes()[..first.len_utf8()];
        let Some(bucket) = self.find_bucket(key) else {
            return Ok(None);
        };
        let start = self.entries[bucket].offset;
        let end = if bucket + 1 < self.count {
            self.entries[bucket + 1].offset
        } else {
            self.dict_len
        };

        // Phase 1: scan the bucket for the target line's bounds. The window is
        // scratch here, so the hit is recorded by offset/length only; that
        // keeps the borrows per-iteration and costs one extra (cheap,
        // sequential) re-read of the matched line in phase 2.
        let mut hit: Option<(u32, usize)> = None;
        let mut offset = start;
        'scan: while offset < end {
            let want = ((end - offset) as usize).min(window.len());
            let n = read_fully(reader, offset, &mut window[..want])?;
            if n == 0 {
                // Dictionary shorter than the index says: treat as a miss.
                return Ok(None);
            }
            let chunk = &window[..n];
            let mut pos = 0;
            while pos < n {
                let line_len = match chunk[pos..].iter().position(|&b| b == b'\n') {
                    Some(i) => i,
                    // The bucket's final line may lack a trailing newline.
                    None if offset + n as u32 >= end => n - pos,
                    // Partial tail line: re-read from its own start.
                    None => break,
                };
                let line = &chunk[pos..pos + line_len];
                if let Some(entry) = parse_entry(line) {
                    match entry.reading.cmp(reading) {
                        core::cmp::Ordering::Less => {}
                        core::cmp::Ordering::Equal => {
                            hit = Some((offset + pos as u32, line_len));
                            break 'scan;
                        }
                        // Sorted dictionary: past the reading means absent.
                        core::cmp::Ordering::Greater => return Ok(None),
                    }
                }
                pos += line_len + 1;
            }
            if pos == 0 {
                // A full window without a newline: overlong line. It can never
                // match (it would not fit the window to return), so skip to
                // the next newline and resume.
                debug_assert_eq!(n, window.len());
                offset += n as u32;
                while offset < end {
                    let n = reader.read_at(offset, window)?;
                    if n == 0 {
                        return Ok(None);
                    }
                    match window[..n].iter().position(|&b| b == b'\n') {
                        Some(i) => {
                            offset += i as u32 + 1;
                            break;
                        }
                        None => offset += n as u32,
                    }
                }
            } else {
                offset += pos.min(n) as u32;
            }
        }

        // Phase 2: bring the matched line back into the window so the entry
        // borrows the caller's buffer.
        let Some((line_offset, line_len)) = hit else {
            return Ok(None);
        };
        if read_fully(reader, line_offset, &mut window[..line_len])? < line_len {
            return Err(SkkError::Io);
        }
        let window: &'w [u8] = window;
        Ok(parse_entry(&window[..line_len]))
    }

    fn find_bucket(&self, key: &[u8]) -> Option<usize> {
        let mut lo = 0;
        let mut hi = self.count;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.entries[mid].key_slice().cmp(key) {
                core::cmp::Ordering::Less => lo = mid + 1,
                core::cmp::Ordering::Greater => hi = mid,
                core::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }
}

/// One borrowed dictionary entry: a reading and its raw candidate list.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DictEntry<'a> {
    reading: &'a str,
    body: &'a str,
}

impl<'a> DictEntry<'a> {
    /// The reading (kana key) for this entry.
    pub fn reading(&self) -> &'a str {
        self.reading
    }

    /// The raw `/candidate/.../` slice, including any `;annotation` suffixes.
    pub fn raw_candidates(&self) -> &'a str {
        self.body
    }

    /// Iterate candidates, dropping the leading `/`, empties, and `;annotations`.
    pub fn candidates(&self) -> Candidates<'a> {
        Candidates { rest: self.body }
    }
}

/// Iterator over the candidate conversions of a [`DictEntry`].
#[derive(Clone, Debug)]
pub struct Candidates<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Candidates<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        loop {
            self.rest = self.rest.trim_start_matches('/');
            if self.rest.is_empty() {
                return None;
            }
            let end = self.rest.find('/').unwrap_or(self.rest.len());
            let field = &self.rest[..end];
            self.rest = &self.rest[end..];
            // SKK annotations follow a ';' inside a candidate field.
            let candidate = match field.find(';') {
                Some(i) => &field[..i],
                None => field,
            };
            if !candidate.is_empty() {
                return Some(candidate);
            }
        }
    }
}

/// Return the line at `offset` (without its trailing newline) and the next offset.
fn line_at(dict: &[u8], offset: usize) -> (&[u8], usize) {
    let rest = &dict[offset..];
    match rest.iter().position(|&b| b == b'\n') {
        Some(i) => (&rest[..i], offset + i + 1),
        None => (rest, dict.len()),
    }
}

/// Parse one dictionary line into a reading and candidate body.
///
/// Returns `None` for comments, blanks, invalid UTF-8, or malformed lines.
fn parse_entry(line: &[u8]) -> Option<DictEntry<'_>> {
    let line = strip_cr(line);
    let text = core::str::from_utf8(line).ok()?;
    if text.is_empty() || text.starts_with(';') {
        return None;
    }
    let space = text.find(' ')?;
    let reading = &text[..space];
    let body = text[space + 1..].trim();
    if reading.is_empty() || !body.starts_with('/') {
        return None;
    }
    Some(DictEntry { reading, body })
}

/// Drop a single trailing carriage return so CRLF dictionaries parse cleanly.
fn strip_cr(line: &[u8]) -> &[u8] {
    match line {
        [head @ .., b'\r'] => head,
        _ => line,
    }
}

/// Best-effort parse of a truncated line head: the reading of a would-be entry
/// whose tail did not fit the scan window. Mirrors [`parse_entry`]'s accepts
/// for everything visible (not a comment, non-empty reading, body starts with
/// `/`), so an overlong line can still open its index bucket.
fn parse_reading_prefix(head: &[u8]) -> Option<&str> {
    if head.first() == Some(&b';') {
        return None;
    }
    let space = head.iter().position(|&b| b == b' ')?;
    let reading = core::str::from_utf8(&head[..space]).ok()?;
    if reading.is_empty() {
        return None;
    }
    let mut body = space + 1;
    while head.get(body) == Some(&b' ') {
        body += 1;
    }
    if head.get(body) != Some(&b'/') {
        return None;
    }
    Some(reading)
}

/// Uniform dictionary access for the IME conversion path, so
/// [`crate::memo_ime::MemoIme`] converts identically against a resident slice
/// (sim fixtures, tests) and a windowed SD reader (hardware).
pub trait SkkDictAccess {
    /// Look up `reading`; on a hit the entry borrows this access object's
    /// backing storage until the next call.
    fn lookup_reading(&mut self, reading: &str) -> Result<Option<DictEntry<'_>>, SkkError>;
}

/// [`SkkDictAccess`] over a memory-resident dictionary slice.
pub struct SliceDict<'d, const N: usize> {
    pub index: &'d SkkIndex<N>,
    pub dict: &'d [u8],
}

impl<'d, const N: usize> SkkDictAccess for SliceDict<'d, N> {
    fn lookup_reading(&mut self, reading: &str) -> Result<Option<DictEntry<'_>>, SkkError> {
        Ok(self.index.lookup(self.dict, reading))
    }
}

/// [`SkkDictAccess`] over a windowed reader: the dictionary body stays on its
/// storage and only `window` (see [`SKK_LOOKUP_WINDOW_BYTES`]) is resident.
pub struct WindowedDict<'a, R: SkkRead, const N: usize> {
    pub index: &'a SkkIndex<N>,
    pub reader: R,
    pub window: &'a mut [u8],
}

impl<'a, R: SkkRead, const N: usize> SkkDictAccess for WindowedDict<'a, R, N> {
    fn lookup_reading(&mut self, reading: &str) -> Result<Option<DictEntry<'_>>, SkkError> {
        self.index
            .lookup_in_reader(&mut self.reader, reading, &mut *self.window)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DICT: &[u8] = include_bytes!("../../../harness/fixtures/skk_min.skk");

    fn candidates<'a>(entry: &DictEntry<'a>) -> std::vec::Vec<&'a str> {
        entry.candidates().collect()
    }

    #[test]
    fn indexes_one_bucket_per_leading_character() {
        let index = SkkLeadingIndex::build(DICT).unwrap();
        // あ, か, き, さ, に, ひ
        assert_eq!(index.entry_count(), 6);
        assert!(index.sram_bytes() < 256);
    }

    #[test]
    fn looks_up_entry_sharing_a_leading_character() {
        let index = SkkLeadingIndex::build(DICT).unwrap();

        let kanji = index.lookup(DICT, "かんじ").unwrap();
        assert_eq!(kanji.reading(), "かんじ");
        assert_eq!(candidates(&kanji), ["漢字", "感じ", "幹事"]);

        // Same bucket, scanned past the first entry.
        let kai = index.lookup(DICT, "かい").unwrap();
        assert_eq!(candidates(&kai), ["回", "会", "貝"]);
        let kasa = index.lookup(DICT, "かさ").unwrap();
        assert_eq!(candidates(&kasa), ["傘", "笠"]);
    }

    #[test]
    fn strips_annotations_from_candidates() {
        let index = SkkLeadingIndex::build(DICT).unwrap();
        let kyou = index.lookup(DICT, "きょう").unwrap();
        assert_eq!(candidates(&kyou), ["今日", "京", "強"]);
        assert_eq!(kyou.raw_candidates(), "/今日;today/京/強/");
    }

    #[test]
    fn looks_up_first_and_last_buckets() {
        let index = SkkLeadingIndex::build(DICT).unwrap();
        assert_eq!(
            candidates(&index.lookup(DICT, "あい").unwrap()),
            ["愛", "相", "藍"]
        );
        assert_eq!(
            candidates(&index.lookup(DICT, "ひらがな").unwrap()),
            ["平仮名"]
        );
    }

    #[test]
    fn misses_unknown_leading_character() {
        let index = SkkLeadingIndex::build(DICT).unwrap();
        assert!(index.lookup(DICT, "こ").is_none());
        assert!(index.lookup(DICT, "").is_none());
    }

    #[test]
    fn misses_unknown_reading_within_a_known_bucket() {
        let index = SkkLeadingIndex::build(DICT).unwrap();
        // Leading 'か' exists; scan stops once readings sort past the target.
        assert!(index.lookup(DICT, "かこ").is_none());
        assert!(index.lookup(DICT, "かんじゃ").is_none());
    }

    #[test]
    fn reports_index_overflow() {
        // The fixture has six buckets; a two-slot index cannot hold them.
        assert_eq!(SkkIndex::<2>::build(DICT), Err(SkkError::IndexFull));
    }

    // The shipped okuri-nasi dictionary (KOTO-0089). Guards the packaging
    // assumptions for hand-edited entries: ascending byte-lexicographic sort,
    // default index capacity, and the line format the parser expects.
    const KOTO_DICT: &[u8] = include_bytes!("../../../sdcard_mock/dict/skk_koto.skk");

    #[test]
    fn koto_dict_is_sorted_and_every_reading_is_reachable() {
        let index = SkkLeadingIndex::build(KOTO_DICT).unwrap();
        assert!(index.entry_count() <= DEFAULT_INDEX_CAPACITY);
        let mut prev: Option<&str> = None;
        for raw in KOTO_DICT.split(|&b| b == b'\n') {
            let raw = strip_cr(raw);
            if raw.is_empty() || raw[0] == b';' {
                continue;
            }
            let line = core::str::from_utf8(raw).unwrap();
            let (reading, _) = line.split_once(' ').unwrap();
            if let Some(prev) = prev {
                assert!(prev < reading, "unsorted readings: {prev} then {reading}");
            }
            prev = Some(reading);
            let entry = index.lookup(KOTO_DICT, reading).unwrap();
            assert_eq!(entry.reading(), reading);
            assert!(entry.candidates().count() > 0);
        }
    }

    #[test]
    fn koto_dict_serves_everyday_readings() {
        let index = SkkLeadingIndex::build(KOTO_DICT).unwrap();
        let kanji = index.lookup(KOTO_DICT, "かんじ").unwrap();
        assert!(candidates(&kanji).contains(&"漢字"));
        let nihon = index.lookup(KOTO_DICT, "にほん").unwrap();
        assert!(candidates(&nihon).contains(&"日本"));
        let henkan = index.lookup(KOTO_DICT, "へんかん").unwrap();
        assert!(candidates(&henkan).contains(&"変換"));
    }

    /// Every reading in a dictionary, in file order.
    fn readings(dict: &[u8]) -> std::vec::Vec<&str> {
        dict.split(|&b| b == b'\n')
            .filter_map(|line| parse_entry(strip_cr(line)))
            .map(|entry| entry.reading)
            .collect()
    }

    #[test]
    fn streaming_build_matches_slice_build() {
        for dict in [DICT, KOTO_DICT] {
            let expected = SkkLeadingIndex::build(dict).unwrap();
            // 24 forces overlong-line head handling on the fixture; 512 is the
            // real packaging window.
            for window_len in [24, SKK_LOOKUP_WINDOW_BYTES] {
                let mut window = std::vec![0u8; window_len];
                let streamed =
                    SkkLeadingIndex::build_from_reader(&mut &dict[..], &mut window).unwrap();
                assert_eq!(streamed, expected, "window={window_len}");
            }
        }
    }

    #[test]
    fn windowed_lookup_matches_slice_lookup() {
        let index = SkkLeadingIndex::build(DICT).unwrap();
        // 48 forces chunk boundaries and partial-tail re-reads inside buckets;
        // both windows fit every fixture line.
        for window_len in [48, SKK_LOOKUP_WINDOW_BYTES] {
            let mut window = std::vec![0u8; window_len];
            for reading in readings(DICT) {
                let expected: std::vec::Vec<&str> =
                    index.lookup(DICT, reading).unwrap().candidates().collect();
                let entry = index
                    .lookup_in_reader(&mut &DICT[..], reading, &mut window)
                    .unwrap()
                    .unwrap();
                assert_eq!(candidates(&entry), expected, "window={window_len}");
            }
            for miss in ["かこ", "かんじゃ", "こ", ""] {
                assert!(index
                    .lookup_in_reader(&mut &DICT[..], miss, &mut window)
                    .unwrap()
                    .is_none());
            }
        }
    }

    #[test]
    fn windowed_lookup_skips_lines_longer_than_the_window() {
        // An 8-byte window cannot hold any fixture line; lookups must miss
        // cleanly (skip-forward path) rather than hang or error.
        let index = SkkLeadingIndex::build(DICT).unwrap();
        let mut window = [0u8; 8];
        for reading in ["あい", "かんじ", "ひらがな"] {
            assert_eq!(
                index
                    .lookup_in_reader(&mut &DICT[..], reading, &mut window)
                    .unwrap(),
                None
            );
        }
    }

    #[test]
    fn koto_dict_lines_fit_the_lookup_window() {
        // Packaging contract for `lookup_in_reader`: every line, newline
        // included, must fit `SKK_LOOKUP_WINDOW_BYTES` or it becomes
        // unreachable through the windowed path.
        let longest = KOTO_DICT
            .split(|&b| b == b'\n')
            .map(|line| line.len() + 1)
            .max()
            .unwrap();
        assert!(
            longest <= SKK_LOOKUP_WINDOW_BYTES,
            "longest line {longest} exceeds the {SKK_LOOKUP_WINDOW_BYTES}-byte window"
        );
    }

    #[test]
    fn koto_dict_windowed_path_round_trips_every_reading() {
        let mut window = [0u8; SKK_LOOKUP_WINDOW_BYTES];
        let index = SkkLeadingIndex::build_from_reader(&mut &KOTO_DICT[..], &mut window).unwrap();
        assert_eq!(index, SkkLeadingIndex::build(KOTO_DICT).unwrap());
        for reading in readings(KOTO_DICT) {
            let expected: std::vec::Vec<&str> = index
                .lookup(KOTO_DICT, reading)
                .unwrap()
                .candidates()
                .collect();
            let entry = index
                .lookup_in_reader(&mut &KOTO_DICT[..], reading, &mut window)
                .unwrap()
                .unwrap();
            assert_eq!(candidates(&entry), expected);
        }
    }
}
