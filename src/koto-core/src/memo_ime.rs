use crate::ime::{ImeError, RomajiKanaInput, StickyShift, StickyShiftKey, StickyShiftOutput};
use crate::memo::{MemoEditor, MemoError};
use crate::skk::{SkkDictAccess, SkkError, SkkIndex, SliceDict};

pub const MEMO_IME_READING_CAPACITY: usize = 64;
pub const MEMO_IME_CANDIDATE_CAPACITY: usize = 64;
pub type KotoMemoIme = MemoIme<MEMO_IME_READING_CAPACITY, MEMO_IME_CANDIDATE_CAPACITY>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoImeError {
    Ime(ImeError),
    Memo(MemoError),
    /// The dictionary storage failed mid-lookup (windowed SD access).
    Dict(SkkError),
    ReadingFull,
    CandidateFull,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoImeKey {
    Character(char),
    Shift,
    Convert,
    Commit,
    Cancel,
    Backspace,
    Other,
    Toggle,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MemoImeMode {
    Empty,
    Composing,
    Converting,
    Candidate,
    MissingCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoImeLine<'a> {
    pub mode: MemoImeMode,
    pub pending_romaji: &'a str,
    pub reading: &'a str,
    pub candidate: Option<&'a str>,
    /// Zero-based index of the shown candidate within the conversion's candidate
    /// list (`0` when there is none).
    pub candidate_index: usize,
    /// Total candidates available for the current reading (`0` when not in a
    /// candidate state).
    pub candidate_count: usize,
    pub sticky_shift_armed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoIme<
    const READING: usize = MEMO_IME_READING_CAPACITY,
    const CANDIDATE: usize = MEMO_IME_CANDIDATE_CAPACITY,
> {
    romaji: RomajiKanaInput,
    sticky: StickyShift,
    enabled: bool,
    converting: bool,
    reading: [u8; READING],
    reading_len: usize,
    candidate: [u8; CANDIDATE],
    candidate_len: usize,
    candidate_index: usize,
    candidate_count: usize,
    missing_candidate: bool,
}

impl<const READING: usize, const CANDIDATE: usize> MemoIme<READING, CANDIDATE> {
    pub const fn new() -> Self {
        Self {
            romaji: RomajiKanaInput::new(),
            sticky: StickyShift::new(),
            enabled: false,
            converting: false,
            reading: [0; READING],
            reading_len: 0,
            candidate: [0; CANDIDATE],
            candidate_len: 0,
            candidate_index: 0,
            candidate_count: 0,
            missing_candidate: false,
        }
    }

    pub fn line(&self) -> MemoImeLine<'_> {
        let mode = if self.candidate_len > 0 {
            MemoImeMode::Candidate
        } else if self.missing_candidate {
            MemoImeMode::MissingCandidate
        } else if self.converting {
            MemoImeMode::Converting
        } else if self.romaji.is_composing() {
            MemoImeMode::Composing
        } else {
            MemoImeMode::Empty
        };
        MemoImeLine {
            mode,
            pending_romaji: self.romaji.pending(),
            reading: self.reading(),
            candidate: self.candidate(),
            candidate_index: self.candidate_index,
            candidate_count: self.candidate_count,
            sticky_shift_armed: self.sticky.is_armed(),
        }
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn reading(&self) -> &str {
        core::str::from_utf8(&self.reading[..self.reading_len]).unwrap_or("")
    }

    pub fn candidate(&self) -> Option<&str> {
        if self.candidate_len == 0 {
            return None;
        }
        core::str::from_utf8(&self.candidate[..self.candidate_len]).ok()
    }

    fn has_active_composition(&self) -> bool {
        self.converting || self.reading_len > 0 || self.candidate_len > 0 || self.missing_candidate
    }

    pub fn process_key<const CAP: usize, const DIRTY: usize>(
        &mut self,
        key: MemoImeKey,
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        match key {
            MemoImeKey::Shift => {
                if self.has_active_composition() {
                    return self.commit(editor);
                }
                self.sticky.process(StickyShiftKey::Shift);
                editor.dirty_mut().mark_ime();
                Ok(())
            }
            MemoImeKey::Character(ch) => {
                let ch = match self.sticky.process(StickyShiftKey::Character(ch)) {
                    StickyShiftOutput::Character(ch) => ch,
                    StickyShiftOutput::None => return Ok(()),
                };
                if !self.enabled {
                    editor.insert_char(ch).map_err(MemoImeError::Memo)?;
                    editor.dirty_mut().mark_ime();
                    return Ok(());
                }
                // SKK-style `q`: while converting, commit the reading as katakana.
                if self.converting && (ch == 'q' || ch == 'Q') {
                    self.finish_pending(editor)?;
                    return self.commit_katakana(editor);
                }
                // A long-vowel mark is part of an active SKK reading, not
                // punctuation to insert into the document. PicoCalc keyboards
                // enter it with ASCII `-`; accept a direct `ー` scalar too.
                if self.converting && (ch == '-' || ch == 'ー') {
                    self.finish_pending(editor)?;
                    self.clear_candidate();
                    self.missing_candidate = false;
                    self.push_reading("ー")?;
                    editor.dirty_mut().mark_ime();
                    return Ok(());
                }
                if !ch.is_ascii_alphabetic() && ch != '\'' {
                    self.finish_pending(editor)?;
                    if !self.converting {
                        // IME on: punctuation/symbols become their full-width forms.
                        editor
                            .insert_char(fullwidth_symbol(ch))
                            .map_err(MemoImeError::Memo)?;
                    }
                    editor.dirty_mut().mark_ime();
                    return Ok(());
                }
                if ch.is_ascii_uppercase() {
                    self.begin_conversion();
                }
                self.push_romaji(ch.to_ascii_lowercase(), editor)
            }
            MemoImeKey::Convert => self.convert(editor),
            MemoImeKey::Commit => self.commit(editor),
            MemoImeKey::Cancel => {
                self.cancel();
                editor.dirty_mut().mark_ime();
                Ok(())
            }
            MemoImeKey::Backspace => {
                self.backspace_composition();
                editor.dirty_mut().mark_ime();
                Ok(())
            }
            MemoImeKey::Other => {
                self.sticky.process(StickyShiftKey::Other);
                editor.dirty_mut().mark_ime();
                Ok(())
            }
            MemoImeKey::Toggle => {
                self.enabled = !self.enabled;
                self.cancel();
                editor.dirty_mut().mark_ime();
                Ok(())
            }
        }
    }

    /// Convert against a memory-resident dictionary slice. Thin wrapper over
    /// [`Self::convert_with_access`] for fixtures and hosts that hold the
    /// dictionary in RAM.
    pub fn convert_with<const N: usize, const CAP: usize, const DIRTY: usize>(
        &mut self,
        index: &SkkIndex<N>,
        dict: &[u8],
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        self.convert_with_access(&mut SliceDict { index, dict }, editor)
    }

    /// Convert the current reading through any [`SkkDictAccess`] — a resident
    /// slice or the hardware's windowed SD reader — so both paths share the
    /// candidate-cycling logic below.
    pub fn convert_with_access<A: SkkDictAccess, const CAP: usize, const DIRTY: usize>(
        &mut self,
        dict: &mut A,
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        self.finish_pending(editor)?;
        self.missing_candidate = false;
        if self.reading_len == 0 {
            self.clear_candidate();
            editor.dirty_mut().mark_ime();
            return Ok(());
        }
        let lookup = dict
            .lookup_reading(self.reading())
            .map_err(MemoImeError::Dict);
        match lookup? {
            Some(entry) => {
                let count = entry.candidates().count();
                if count == 0 {
                    self.clear_candidate();
                    self.missing_candidate = true;
                } else {
                    // Re-running conversion on an already shown candidate cycles to
                    // the next one (wrapping); a fresh conversion starts at index 0.
                    self.candidate_index = if self.candidate_len > 0 {
                        (self.candidate_index + 1) % count
                    } else {
                        0
                    };
                    self.candidate_count = count;
                    if let Some(candidate) = entry.candidates().nth(self.candidate_index) {
                        self.set_candidate(candidate)?;
                    }
                }
            }
            None => {
                self.clear_candidate();
                self.missing_candidate = true;
            }
        }
        editor.dirty_mut().mark_ime();
        Ok(())
    }

    fn convert<const CAP: usize, const DIRTY: usize>(
        &mut self,
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        self.finish_pending(editor)?;
        self.missing_candidate = self.reading_len > 0;
        editor.dirty_mut().mark_ime();
        Ok(())
    }

    fn commit<const CAP: usize, const DIRTY: usize>(
        &mut self,
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        self.finish_pending(editor)?;
        if let Some(candidate) = self.candidate() {
            editor.insert_str(candidate).map_err(MemoImeError::Memo)?;
        } else if self.reading_len > 0 {
            let reading = self.reading();
            editor.insert_str(reading).map_err(MemoImeError::Memo)?;
        }
        self.cancel();
        editor.dirty_mut().mark_ime();
        Ok(())
    }

    fn push_romaji<const CAP: usize, const DIRTY: usize>(
        &mut self,
        ch: char,
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        self.clear_candidate();
        self.missing_candidate = false;
        match self.romaji.push(ch) {
            Ok(output) if output.committed_text().is_some() => {
                let kana = output.committed_text().unwrap_or("");
                if self.converting {
                    self.push_reading(kana)?;
                } else {
                    editor.insert_str(kana).map_err(MemoImeError::Memo)?;
                }
            }
            Ok(_) => {}
            Err(ImeError::InvalidSequence | ImeError::UnsupportedInput | ImeError::BufferFull) => {
                self.recover_invalid_romaji(ch, editor)?;
            }
            Err(error) => return Err(MemoImeError::Ime(error)),
        }
        editor.dirty_mut().mark_ime();
        Ok(())
    }

    fn recover_invalid_romaji<const CAP: usize, const DIRTY: usize>(
        &mut self,
        ch: char,
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        let mut raw = [0u8; MAX_RECOVERY_BYTES];
        let mut len = 0usize;
        for byte in self.romaji.pending().bytes() {
            if len < raw.len() {
                raw[len] = byte;
                len += 1;
            }
        }
        let mut ch_buf = [0u8; 4];
        let ch_text = ch.encode_utf8(&mut ch_buf);
        for byte in ch_text.bytes() {
            if len < raw.len() {
                raw[len] = byte;
                len += 1;
            }
        }
        self.romaji.reset();
        let text = core::str::from_utf8(&raw[..len])
            .map_err(|_| MemoImeError::Ime(ImeError::InvalidSequence))?;
        if self.converting {
            self.push_reading(text)
        } else {
            editor.insert_str(text).map_err(MemoImeError::Memo)
        }
    }

    fn finish_pending<const CAP: usize, const DIRTY: usize>(
        &mut self,
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        if let Some(kana) = self
            .romaji
            .finish()
            .map_err(MemoImeError::Ime)?
            .committed_text()
        {
            if self.converting {
                self.push_reading(kana)?;
            } else {
                editor.insert_str(kana).map_err(MemoImeError::Memo)?;
            }
        }
        Ok(())
    }

    fn begin_conversion(&mut self) {
        self.converting = true;
        self.reading_len = 0;
        self.clear_candidate();
        self.missing_candidate = false;
        self.romaji.reset();
    }

    fn cancel(&mut self) {
        self.romaji.reset();
        self.sticky.reset();
        self.converting = false;
        self.reading_len = 0;
        self.clear_candidate();
        self.missing_candidate = false;
    }

    fn push_reading(&mut self, text: &str) -> Result<(), MemoImeError> {
        if self.reading_len + text.len() > READING {
            return Err(MemoImeError::ReadingFull);
        }
        self.reading[self.reading_len..self.reading_len + text.len()]
            .copy_from_slice(text.as_bytes());
        self.reading_len += text.len();
        Ok(())
    }

    fn set_candidate(&mut self, text: &str) -> Result<(), MemoImeError> {
        if text.len() > CANDIDATE {
            return Err(MemoImeError::CandidateFull);
        }
        self.candidate[..text.len()].copy_from_slice(text.as_bytes());
        self.candidate_len = text.len();
        Ok(())
    }

    fn clear_candidate(&mut self) {
        self.candidate_len = 0;
        self.candidate_index = 0;
        self.candidate_count = 0;
    }

    /// Commit the current reading converted to katakana, then clear composition.
    fn commit_katakana<const CAP: usize, const DIRTY: usize>(
        &mut self,
        editor: &mut MemoEditor<CAP, DIRTY>,
    ) -> Result<(), MemoImeError> {
        for ch in self.reading().chars() {
            editor
                .insert_char(hiragana_to_katakana(ch))
                .map_err(MemoImeError::Memo)?;
        }
        self.cancel();
        editor.dirty_mut().mark_ime();
        Ok(())
    }

    /// Backspace inside an active composition: drop the last pending romaji byte,
    /// else the last reading character, and clear any shown candidate. When no
    /// pending romaji or reading remains, the composition ends.
    fn backspace_composition(&mut self) {
        if self.romaji.is_composing() {
            self.romaji.backspace();
        } else if self.reading_len > 0 {
            self.pop_reading_char();
        }
        self.clear_candidate();
        self.missing_candidate = false;
        if !self.romaji.is_composing() && self.reading_len == 0 {
            self.converting = false;
            self.sticky.reset();
        }
    }

    fn pop_reading_char(&mut self) {
        if self.reading_len == 0 {
            return;
        }
        let mut at = self.reading_len - 1;
        while at > 0 && (self.reading[at] & 0xC0) == 0x80 {
            at -= 1;
        }
        self.reading_len = at;
    }
}

const MAX_RECOVERY_BYTES: usize = crate::ime::MAX_ROMAJI_BUFFER + 4;

/// Map an ASCII punctuation/symbol to the full-width form a Japanese IME inserts
/// while enabled. `,`/`.` become Japanese punctuation, `-` becomes the katakana
/// long-vowel mark, `/` becomes the middle dot, and the rest of the printable
/// ASCII symbols map to their full-width (U+FF01..U+FF5E) counterparts.
/// Alphanumerics, spaces, newlines, and other control characters are returned
/// unchanged (a memo keeps half-width spacing and digits).
fn fullwidth_symbol(ch: char) -> char {
    match ch {
        ',' => '、',
        '.' => '。',
        '-' => 'ー',
        '/' => '・',
        c if c.is_ascii_graphic() && !c.is_ascii_alphanumeric() => {
            char::from_u32(c as u32 + 0xFEE0).unwrap_or(c)
        }
        _ => ch,
    }
}

/// Map a hiragana scalar to its katakana counterpart; other characters (the long
/// vowel mark `ー`, punctuation) are returned unchanged.
fn hiragana_to_katakana(ch: char) -> char {
    let cp = ch as u32;
    if (0x3041..=0x3096).contains(&cp) {
        char::from_u32(cp + 0x60).unwrap_or(ch)
    } else {
        ch
    }
}

impl<const READING: usize, const CANDIDATE: usize> Default for MemoIme<READING, CANDIDATE> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hal::PixelFormat;
    use crate::layout::{CellMetrics, TextLayout};
    use crate::memo::MemoEditor;
    use crate::render::RenderSurface;
    use crate::skk::SkkLeadingIndex;

    const DICT: &[u8] = include_bytes!("../../../harness/fixtures/skk_min.skk");
    type TestIme = MemoIme<MEMO_IME_READING_CAPACITY, MEMO_IME_CANDIDATE_CAPACITY>;

    fn editor() -> MemoEditor<128> {
        MemoEditor::new(
            TextLayout::new(
                RenderSurface::new(320, 320, PixelFormat::Rgb565),
                CellMetrics::FONT_8X12,
                2,
            )
            .unwrap(),
        )
    }

    #[test]
    fn scripted_romaji_commits_kana_into_memo() {
        let mut editor = editor();
        editor.take_dirty();
        let mut ime = TestIme::new();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();

        for key in ['k', 'a', 's', 'a', 'g', 'o'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }

        assert_eq!(editor.as_str(), "かさご");
        let line = ime.line();
        assert_eq!(line.mode, MemoImeMode::Empty);
        assert!(editor.dirty().ime_dirty());
    }

    #[test]
    fn sticky_shift_starts_skk_conversion_without_chord() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        let index = SkkLeadingIndex::build(DICT).unwrap();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();

        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        assert!(ime.line().sticky_shift_armed);
        for key in ['k', 'a', 's', 'a'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }
        assert_eq!(editor.as_str(), "");
        assert_eq!(ime.line().mode, MemoImeMode::Converting);
        assert_eq!(ime.line().reading, "かさ");

        ime.convert_with(&index, DICT, &mut editor).unwrap();
        assert_eq!(ime.line().mode, MemoImeMode::Candidate);
        assert_eq!(ime.line().candidate, Some("傘"));

        ime.process_key(MemoImeKey::Commit, &mut editor).unwrap();
        assert_eq!(editor.as_str(), "傘");
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
    }

    #[test]
    fn repeated_convert_cycles_through_candidates() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        let index = SkkLeadingIndex::build(DICT).unwrap();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        for key in ['k', 'a', 's', 'a'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }

        // かさ -> /傘/笠/ : first convert shows the first candidate.
        ime.convert_with(&index, DICT, &mut editor).unwrap();
        let line = ime.line();
        assert_eq!(line.candidate, Some("傘"));
        assert_eq!(line.candidate_index, 0);
        assert_eq!(line.candidate_count, 2);

        // Converting again advances to the next candidate.
        ime.convert_with(&index, DICT, &mut editor).unwrap();
        let line = ime.line();
        assert_eq!(line.candidate, Some("笠"));
        assert_eq!(line.candidate_index, 1);
        assert_eq!(line.candidate_count, 2);

        // And wraps back to the first.
        ime.convert_with(&index, DICT, &mut editor).unwrap();
        assert_eq!(ime.line().candidate, Some("傘"));
        assert_eq!(ime.line().candidate_index, 0);

        // Commit inserts whichever candidate is shown.
        ime.convert_with(&index, DICT, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Commit, &mut editor).unwrap();
        assert_eq!(editor.as_str(), "笠");
    }

    #[test]
    fn windowed_access_converts_and_cycles_like_the_slice_path() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        let index = SkkLeadingIndex::build(DICT).unwrap();
        // A 48-byte window forces multi-chunk bucket scans, mirroring the
        // hardware's SD-windowed conversion path end to end.
        let mut window = [0u8; 48];
        let mut access = crate::skk::WindowedDict {
            index: &index,
            reader: DICT,
            window: &mut window,
        };
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        for key in ['k', 'a', 's', 'a'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }

        ime.convert_with_access(&mut access, &mut editor).unwrap();
        assert_eq!(ime.line().candidate, Some("傘"));
        ime.convert_with_access(&mut access, &mut editor).unwrap();
        assert_eq!(ime.line().candidate, Some("笠"));
        ime.process_key(MemoImeKey::Commit, &mut editor).unwrap();
        assert_eq!(editor.as_str(), "笠");
    }

    #[test]
    fn backspace_edits_reading_then_ends_conversion() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        let index = SkkLeadingIndex::build(DICT).unwrap();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        for key in ['k', 'a', 's', 'a'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }
        ime.convert_with(&index, DICT, &mut editor).unwrap();
        assert_eq!(ime.line().mode, MemoImeMode::Candidate);

        // Backspace drops the candidate and the last reading character (かさ -> か),
        // staying in conversion without writing to the document.
        ime.process_key(MemoImeKey::Backspace, &mut editor).unwrap();
        assert_eq!(ime.line().mode, MemoImeMode::Converting);
        assert_eq!(ime.line().reading, "か");
        assert_eq!(editor.as_str(), "");

        // Emptying the reading ends the conversion entirely.
        ime.process_key(MemoImeKey::Backspace, &mut editor).unwrap();
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
        assert_eq!(editor.as_str(), "");
    }

    #[test]
    fn ime_on_inserts_fullwidth_symbols_but_off_stays_ascii() {
        let mut editor = editor();
        let mut ime = TestIme::new();

        // IME off: symbols stay half-width.
        for ch in ['a', ',', '!'] {
            ime.process_key(MemoImeKey::Character(ch), &mut editor)
                .unwrap();
        }
        assert_eq!(editor.as_str(), "a,!");

        // IME on: kana for letters, full-width / Japanese punctuation for symbols.
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        for ch in ['a', ',', '.', '!', '?', '-', '/'] {
            ime.process_key(MemoImeKey::Character(ch), &mut editor)
                .unwrap();
        }
        assert_eq!(editor.as_str(), "a,!あ、。！？ー・");
    }

    #[test]
    fn q_commits_reading_as_katakana_while_converting() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        for key in ['k', 'a', 's', 'a'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }
        assert_eq!(ime.line().reading, "かさ");

        // `q` commits the reading as katakana and ends the composition.
        ime.process_key(MemoImeKey::Character('q'), &mut editor)
            .unwrap();
        assert_eq!(editor.as_str(), "カサ");
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
    }

    #[test]
    fn long_vowel_mark_extends_active_conversion_reading() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        for key in ['k', 'o', 'n', 'p', 'y', 'u', '-', 't', 'a'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }

        assert_eq!(ime.line().reading, "こんぴゅーた");
        assert_eq!(editor.as_str(), "");

        // SKK `q` commits the active reading as katakana.
        ime.process_key(MemoImeKey::Character('q'), &mut editor)
            .unwrap();
        assert_eq!(editor.as_str(), "コンピュータ");
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
    }

    #[test]
    fn shift_commits_active_candidate_for_right_shift_fallback() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        let index = SkkLeadingIndex::build(DICT).unwrap();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();

        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        for key in ['k', 'a', 's', 'a'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }
        ime.convert_with(&index, DICT, &mut editor).unwrap();
        assert_eq!(ime.line().mode, MemoImeMode::Candidate);

        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        assert_eq!(editor.as_str(), "傘");
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
    }

    #[test]
    fn missing_candidate_is_visible_and_commits_reading() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        let index = SkkLeadingIndex::build(DICT).unwrap();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();

        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        for key in ['k', 'o'] {
            ime.process_key(MemoImeKey::Character(key), &mut editor)
                .unwrap();
        }
        ime.convert_with(&index, DICT, &mut editor).unwrap();

        assert_eq!(ime.line().mode, MemoImeMode::MissingCandidate);
        assert_eq!(ime.line().reading, "こ");
        ime.process_key(MemoImeKey::Commit, &mut editor).unwrap();
        assert_eq!(editor.as_str(), "こ");
    }

    #[test]
    fn cancel_clears_composition_state() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();

        ime.process_key(MemoImeKey::Character('s'), &mut editor)
            .unwrap();
        assert_eq!(ime.line().mode, MemoImeMode::Composing);
        ime.process_key(MemoImeKey::Cancel, &mut editor).unwrap();
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
        assert_eq!(editor.as_str(), "");
    }

    #[test]
    fn non_romaji_characters_insert_after_finishing_pending_text() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();

        ime.process_key(MemoImeKey::Character('k'), &mut editor)
            .unwrap();
        ime.process_key(MemoImeKey::Character('a'), &mut editor)
            .unwrap();
        ime.process_key(MemoImeKey::Character(' '), &mut editor)
            .unwrap();
        ime.process_key(MemoImeKey::Character('\n'), &mut editor)
            .unwrap();

        assert_eq!(editor.as_str(), "か \n");
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
    }

    #[test]
    fn disabled_ime_inserts_ascii_directly_and_toggle_clears_composition() {
        let mut editor = editor();
        let mut ime = TestIme::new();

        assert!(!ime.is_enabled());
        ime.process_key(MemoImeKey::Character('k'), &mut editor)
            .unwrap();
        assert_eq!(editor.as_str(), "k");

        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        assert!(ime.is_enabled());
        ime.process_key(MemoImeKey::Character('k'), &mut editor)
            .unwrap();
        assert_eq!(ime.line().pending_romaji, "k");
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        assert!(!ime.is_enabled());
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
    }

    #[test]
    fn invalid_romaji_recovers_pending_as_ascii_without_losing_existing_text() {
        let mut editor = editor();
        let mut ime = TestIme::new();

        editor.insert_str("abc").unwrap();
        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Character('k'), &mut editor)
            .unwrap();
        ime.process_key(MemoImeKey::Character('x'), &mut editor)
            .unwrap();

        assert_eq!(editor.as_str(), "abckx");
        assert_eq!(ime.line().mode, MemoImeMode::Empty);
    }

    #[test]
    fn invalid_romaji_inside_conversion_stays_visible_as_missing_reading() {
        let mut editor = editor();
        let mut ime = TestIme::new();
        let index = SkkLeadingIndex::build(DICT).unwrap();

        ime.process_key(MemoImeKey::Toggle, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Shift, &mut editor).unwrap();
        ime.process_key(MemoImeKey::Character('k'), &mut editor)
            .unwrap();
        ime.process_key(MemoImeKey::Character('x'), &mut editor)
            .unwrap();
        assert_eq!(ime.line().reading, "kx");
        ime.convert_with(&index, DICT, &mut editor).unwrap();

        assert_eq!(ime.line().mode, MemoImeMode::MissingCandidate);
        assert_eq!(editor.as_str(), "");
    }
}
