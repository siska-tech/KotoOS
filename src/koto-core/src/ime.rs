//! Small romaji-to-kana composition core for KotoIME.
//!
//! The state machine is intentionally compact and allocation-free: callers feed
//! one ASCII romaji character at a time, inspect committed kana after each
//! stroke, and render [`RomajiKanaInput::pending`] when composition is still
//! incomplete.

pub const MAX_ROMAJI_BUFFER: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ImeError {
    UnsupportedInput,
    InvalidSequence,
    BufferFull,
    IncompleteComposition,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImeOutput {
    committed: Option<&'static str>,
}

impl ImeOutput {
    pub const NONE: ImeOutput = ImeOutput { committed: None };

    pub const fn committed(kana: &'static str) -> Self {
        Self {
            committed: Some(kana),
        }
    }

    pub const fn committed_text(&self) -> Option<&'static str> {
        self.committed
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StickyShiftKey {
    Shift,
    Character(char),
    Other,
    Cancel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StickyShiftOutput {
    None,
    Character(char),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StickyShift {
    armed: bool,
}

impl StickyShift {
    pub const fn new() -> Self {
        Self { armed: false }
    }

    pub const fn is_armed(&self) -> bool {
        self.armed
    }

    pub fn reset(&mut self) {
        self.armed = false;
    }

    pub fn press_shift(&mut self) {
        self.armed = true;
    }

    pub fn apply_to_char(&mut self, ch: char) -> char {
        if self.armed {
            self.armed = false;
            ch.to_ascii_uppercase()
        } else {
            ch
        }
    }

    pub fn process(&mut self, key: StickyShiftKey) -> StickyShiftOutput {
        match key {
            StickyShiftKey::Shift => {
                self.press_shift();
                StickyShiftOutput::None
            }
            StickyShiftKey::Character(ch) => StickyShiftOutput::Character(self.apply_to_char(ch)),
            StickyShiftKey::Other | StickyShiftKey::Cancel => {
                self.reset();
                StickyShiftOutput::None
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RomajiKanaInput {
    pending: [u8; MAX_ROMAJI_BUFFER],
    pending_len: usize,
}

impl Default for RomajiKanaInput {
    fn default() -> Self {
        Self::new()
    }
}

impl RomajiKanaInput {
    pub const fn new() -> Self {
        Self {
            pending: [0; MAX_ROMAJI_BUFFER],
            pending_len: 0,
        }
    }

    pub fn pending(&self) -> &str {
        core::str::from_utf8(&self.pending[..self.pending_len]).unwrap_or("")
    }

    pub const fn is_composing(&self) -> bool {
        self.pending_len != 0
    }

    pub fn reset(&mut self) {
        self.pending_len = 0;
    }

    /// Remove the last pending romaji byte (each romaji key is one ASCII byte), for
    /// editing an in-progress composition with Backspace. Returns `true` if a byte
    /// was removed.
    pub fn backspace(&mut self) -> bool {
        let had = self.pending_len > 0;
        self.pop_byte();
        had
    }

    pub fn push(&mut self, ch: char) -> Result<ImeOutput, ImeError> {
        let ch = normalize_romaji(ch)?;

        if self.pending() == "n" {
            // `nn` and `n'` finish the syllabic ん, consuming both keys (no stray
            // `n` is left pending). `n` before another consonant commits ん and
            // carries that consonant into the next syllable.
            if ch == b'\'' || ch == b'n' {
                self.reset();
                return Ok(ImeOutput::committed("ん"));
            }
            if is_consonant(ch) && ch != b'y' {
                self.reset();
                self.push_byte(ch)?;
                return Ok(ImeOutput::committed("ん"));
            }
        }

        if self.pending_len == 1 && self.pending[0] == ch && is_sokuon_consonant(ch) {
            self.reset();
            self.push_byte(ch)?;
            return Ok(ImeOutput::committed("っ"));
        }

        self.push_byte(ch)?;
        let pending = self.pending();
        if let Some(kana) = kana_for(pending) {
            self.reset();
            return Ok(ImeOutput::committed(kana));
        }
        if is_romaji_prefix(pending) {
            return Ok(ImeOutput::NONE);
        }

        self.pop_byte();
        Err(ImeError::InvalidSequence)
    }

    pub fn finish(&mut self) -> Result<ImeOutput, ImeError> {
        if self.pending_len == 0 {
            return Ok(ImeOutput::NONE);
        }
        if self.pending() == "n" {
            self.reset();
            return Ok(ImeOutput::committed("ん"));
        }
        Err(ImeError::IncompleteComposition)
    }

    fn push_byte(&mut self, ch: u8) -> Result<(), ImeError> {
        if self.pending_len == MAX_ROMAJI_BUFFER {
            return Err(ImeError::BufferFull);
        }
        self.pending[self.pending_len] = ch;
        self.pending_len += 1;
        Ok(())
    }

    fn pop_byte(&mut self) {
        if self.pending_len > 0 {
            self.pending_len -= 1;
        }
    }
}

fn normalize_romaji(ch: char) -> Result<u8, ImeError> {
    if ch == '\'' {
        return Ok(b'\'');
    }
    if ch.is_ascii_alphabetic() {
        return Ok(ch.to_ascii_lowercase() as u8);
    }
    Err(ImeError::UnsupportedInput)
}

fn is_vowel(ch: u8) -> bool {
    matches!(ch, b'a' | b'i' | b'u' | b'e' | b'o')
}

fn is_consonant(ch: u8) -> bool {
    ch.is_ascii_lowercase() && !is_vowel(ch)
}

fn is_sokuon_consonant(ch: u8) -> bool {
    matches!(ch, b'k' | b's' | b't' | b'p')
}

fn kana_for(romaji: &str) -> Option<&'static str> {
    ROMAJI_KANA
        .iter()
        .find_map(|(key, kana)| (*key == romaji).then_some(*kana))
}

fn is_romaji_prefix(prefix: &str) -> bool {
    prefix == "n"
        || ROMAJI_KANA
            .iter()
            .any(|(key, _)| key.starts_with(prefix) && *key != prefix)
}

const ROMAJI_KANA: &[(&str, &str)] = &[
    // ---------------------------------------------------------------------
    // Vowels
    // ---------------------------------------------------------------------
    ("a", "あ"),
    ("i", "い"),
    ("u", "う"),
    ("e", "え"),
    ("o", "お"),
    // ---------------------------------------------------------------------
    // Small vowels
    // ---------------------------------------------------------------------
    ("xa", "ぁ"),
    ("xi", "ぃ"),
    ("xu", "ぅ"),
    ("xe", "ぇ"),
    ("xo", "ぉ"),
    ("la", "ぁ"),
    ("li", "ぃ"),
    ("lu", "ぅ"),
    ("le", "ぇ"),
    ("lo", "ぉ"),
    // ---------------------------------------------------------------------
    // K / G
    // ---------------------------------------------------------------------
    ("ka", "か"),
    ("ki", "き"),
    ("ku", "く"),
    ("ke", "け"),
    ("ko", "こ"),
    ("ga", "が"),
    ("gi", "ぎ"),
    ("gu", "ぐ"),
    ("ge", "げ"),
    ("go", "ご"),
    // ---------------------------------------------------------------------
    // S / Z
    // ---------------------------------------------------------------------
    ("sa", "さ"),
    ("shi", "し"),
    ("si", "し"),
    ("su", "す"),
    ("se", "せ"),
    ("so", "そ"),
    ("za", "ざ"),
    ("ji", "じ"),
    ("zi", "じ"),
    ("zu", "ず"),
    ("ze", "ぜ"),
    ("zo", "ぞ"),
    // ---------------------------------------------------------------------
    // T / D
    // ---------------------------------------------------------------------
    ("ta", "た"),
    ("chi", "ち"),
    ("ti", "ち"),
    ("tsu", "つ"),
    ("tu", "つ"),
    ("te", "て"),
    ("to", "と"),
    ("da", "だ"),
    ("di", "ぢ"),
    ("du", "づ"),
    ("de", "で"),
    ("do", "ど"),
    // ---------------------------------------------------------------------
    // N
    // NOTE:
    // - Do not map bare "n" here.
    // - Bare n should become "ん" only when the next input is not a vowel/y,
    //   or when the composition is committed.
    // ---------------------------------------------------------------------
    ("na", "な"),
    ("ni", "に"),
    ("nu", "ぬ"),
    ("ne", "ね"),
    ("no", "の"),
    // ---------------------------------------------------------------------
    // H / B / P
    // ---------------------------------------------------------------------
    ("ha", "は"),
    ("hi", "ひ"),
    ("fu", "ふ"),
    ("hu", "ふ"),
    ("he", "へ"),
    ("ho", "ほ"),
    ("ba", "ば"),
    ("bi", "び"),
    ("bu", "ぶ"),
    ("be", "べ"),
    ("bo", "ぼ"),
    ("pa", "ぱ"),
    ("pi", "ぴ"),
    ("pu", "ぷ"),
    ("pe", "ぺ"),
    ("po", "ぽ"),
    // ---------------------------------------------------------------------
    // M
    // ---------------------------------------------------------------------
    ("ma", "ま"),
    ("mi", "み"),
    ("mu", "む"),
    ("me", "め"),
    ("mo", "も"),
    // ---------------------------------------------------------------------
    // Y + small Y
    // ---------------------------------------------------------------------
    ("ya", "や"),
    ("yu", "ゆ"),
    ("yo", "よ"),
    ("xya", "ゃ"),
    ("xyu", "ゅ"),
    ("xyo", "ょ"),
    ("lya", "ゃ"),
    ("lyu", "ゅ"),
    ("lyo", "ょ"),
    // ---------------------------------------------------------------------
    // R
    // ---------------------------------------------------------------------
    ("ra", "ら"),
    ("ri", "り"),
    ("ru", "る"),
    ("re", "れ"),
    ("ro", "ろ"),
    // ---------------------------------------------------------------------
    // W
    // ---------------------------------------------------------------------
    ("wa", "わ"),
    ("wo", "を"),
    ("xwa", "ゎ"),
    ("lwa", "ゎ"),
    ("wi", "うぃ"),
    ("we", "うぇ"),
    ("wha", "うぁ"),
    ("whi", "うぃ"),
    ("whu", "う"),
    ("whe", "うぇ"),
    ("who", "うぉ"),
    // ---------------------------------------------------------------------
    // Small tsu
    // NOTE:
    // - These are explicit inputs.
    // - Double-consonant small-tsu, e.g. "kka" -> "っか",
    //   should be handled by a separate rule.
    // ---------------------------------------------------------------------
    ("xtsu", "っ"),
    ("xtu", "っ"),
    ("ltsu", "っ"),
    ("ltu", "っ"),
    // ---------------------------------------------------------------------
    // Basic yoon: K/G
    // ---------------------------------------------------------------------
    ("kya", "きゃ"),
    ("kyu", "きゅ"),
    ("kyo", "きょ"),
    ("gya", "ぎゃ"),
    ("gyu", "ぎゅ"),
    ("gyo", "ぎょ"),
    // ---------------------------------------------------------------------
    // Basic yoon: S/Z/J
    // ---------------------------------------------------------------------
    ("sha", "しゃ"),
    ("shu", "しゅ"),
    ("sho", "しょ"),
    ("sya", "しゃ"),
    ("syu", "しゅ"),
    ("syo", "しょ"),
    ("ja", "じゃ"),
    ("ju", "じゅ"),
    ("jo", "じょ"),
    ("jya", "じゃ"),
    ("jyu", "じゅ"),
    ("jyo", "じょ"),
    // ---------------------------------------------------------------------
    // Basic yoon: T/C/D
    // ---------------------------------------------------------------------
    ("cha", "ちゃ"),
    ("chu", "ちゅ"),
    ("cho", "ちょ"),
    ("cya", "ちゃ"),
    ("cyu", "ちゅ"),
    ("cyo", "ちょ"),
    ("tya", "ちゃ"),
    ("tyu", "ちゅ"),
    ("tyo", "ちょ"),
    ("dya", "ぢゃ"),
    ("dyu", "ぢゅ"),
    ("dyo", "ぢょ"),
    // ---------------------------------------------------------------------
    // Basic yoon: N/H/B/P/M/R
    // ---------------------------------------------------------------------
    ("nya", "にゃ"),
    ("nyu", "にゅ"),
    ("nyo", "にょ"),
    ("hya", "ひゃ"),
    ("hyu", "ひゅ"),
    ("hyo", "ひょ"),
    ("bya", "びゃ"),
    ("byu", "びゅ"),
    ("byo", "びょ"),
    ("pya", "ぴゃ"),
    ("pyu", "ぴゅ"),
    ("pyo", "ぴょ"),
    ("mya", "みゃ"),
    ("myu", "みゅ"),
    ("myo", "みょ"),
    ("rya", "りゃ"),
    ("ryu", "りゅ"),
    ("ryo", "りょ"),
    // ---------------------------------------------------------------------
    // Extended sounds: SH/J/CH
    // ---------------------------------------------------------------------
    ("she", "しぇ"),
    ("je", "じぇ"),
    ("che", "ちぇ"),
    // ---------------------------------------------------------------------
    // Extended sounds: TS/T/D
    // ---------------------------------------------------------------------
    ("tsa", "つぁ"),
    ("tsi", "つぃ"),
    ("tse", "つぇ"),
    ("tso", "つぉ"),
    ("thi", "てぃ"),
    ("thu", "てゅ"),
    ("dhi", "でぃ"),
    ("dhu", "でゅ"),
    // ---------------------------------------------------------------------
    // Extended sounds: F/V
    // ---------------------------------------------------------------------
    ("fa", "ふぁ"),
    ("fi", "ふぃ"),
    ("fe", "ふぇ"),
    ("fo", "ふぉ"),
    ("fya", "ふゃ"),
    ("fyu", "ふゅ"),
    ("fyo", "ふょ"),
    ("va", "ゔぁ"),
    ("vi", "ゔぃ"),
    ("vu", "ゔ"),
    ("ve", "ゔぇ"),
    ("vo", "ゔぉ"),
    // ---------------------------------------------------------------------
    // Extended sounds: KW/GW/Q/SW/ZW
    // ---------------------------------------------------------------------
    ("kwa", "くぁ"),
    ("kwi", "くぃ"),
    ("kwe", "くぇ"),
    ("kwo", "くぉ"),
    ("gwa", "ぐぁ"),
    ("gwi", "ぐぃ"),
    ("gwe", "ぐぇ"),
    ("gwo", "ぐぉ"),
    ("qa", "くぁ"),
    ("qi", "くぃ"),
    ("qe", "くぇ"),
    ("qo", "くぉ"),
    ("swi", "すぃ"),
    ("zwi", "ずぃ"),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn push_all(
        ime: &mut RomajiKanaInput,
        input: &str,
    ) -> Result<heapless_string::String, ImeError> {
        let mut committed = heapless_string::String::new();
        for ch in input.chars() {
            if let Some(kana) = ime.push(ch)?.committed_text() {
                committed.push_str(kana);
            }
        }
        Ok(committed)
    }

    mod heapless_string {
        pub type String = std::string::String;
    }

    #[test]
    fn converts_common_syllables() {
        let mut ime = RomajiKanaInput::new();

        assert_eq!(push_all(&mut ime, "ka").unwrap(), "か");
        assert_eq!(ime.pending(), "");
        assert_eq!(push_all(&mut ime, "shi").unwrap(), "し");
        assert_eq!(ime.finish().unwrap().committed_text(), None);
    }

    #[test]
    fn converts_voiced_and_semi_voiced_syllables() {
        let mut ime = RomajiKanaInput::new();

        assert_eq!(push_all(&mut ime, "gogagiguge").unwrap(), "ごがぎぐげ");
        assert_eq!(push_all(&mut ime, "zazuzozedo").unwrap(), "ざずぞぜど");
        assert_eq!(
            push_all(&mut ime, "babibubebopapipupepo").unwrap(),
            "ばびぶべぼぱぴぷぺぽ"
        );
        assert_eq!(
            push_all(&mut ime, "gyajubyopyu").unwrap(),
            "ぎゃじゅびょぴゅ"
        );
    }

    #[test]
    fn converts_all_standard_youon_rows() {
        // Every consonant + small ゃ/ゅ/ょ row, including き/ひ/み which were once
        // missing (KOTO-0100). Each composes from its full romaji and leaves no
        // pending bytes.
        let mut ime = RomajiKanaInput::new();
        assert_eq!(push_all(&mut ime, "kyakyukyo").unwrap(), "きゃきゅきょ");
        assert_eq!(push_all(&mut ime, "gyagyugyo").unwrap(), "ぎゃぎゅぎょ");
        assert_eq!(push_all(&mut ime, "shashusho").unwrap(), "しゃしゅしょ");
        assert_eq!(push_all(&mut ime, "jajujo").unwrap(), "じゃじゅじょ");
        assert_eq!(push_all(&mut ime, "chachucho").unwrap(), "ちゃちゅちょ");
        assert_eq!(push_all(&mut ime, "nyanyunyo").unwrap(), "にゃにゅにょ");
        assert_eq!(push_all(&mut ime, "hyahyuhyo").unwrap(), "ひゃひゅひょ");
        assert_eq!(push_all(&mut ime, "byabyubyo").unwrap(), "びゃびゅびょ");
        assert_eq!(push_all(&mut ime, "pyapyupyo").unwrap(), "ぴゃぴゅぴょ");
        assert_eq!(push_all(&mut ime, "myamyumyo").unwrap(), "みゃみゅみょ");
        assert_eq!(push_all(&mut ime, "ryaryuryo").unwrap(), "りゃりゅりょ");
        assert_eq!(ime.pending(), "");
    }

    #[test]
    fn keeps_incomplete_composition_buffered() {
        let mut ime = RomajiKanaInput::new();

        assert_eq!(ime.push('s').unwrap().committed_text(), None);
        assert_eq!(ime.pending(), "s");
        assert_eq!(ime.push('h').unwrap().committed_text(), None);
        assert_eq!(ime.pending(), "sh");
        assert_eq!(ime.push('i').unwrap().committed_text(), Some("し"));
        assert_eq!(ime.pending(), "");
    }

    #[test]
    fn converts_n_at_boundaries() {
        let mut ime = RomajiKanaInput::new();

        assert_eq!(ime.push('n').unwrap().committed_text(), None);
        assert_eq!(ime.pending(), "n");
        assert_eq!(ime.finish().unwrap().committed_text(), Some("ん"));
        assert_eq!(ime.pending(), "");

        assert_eq!(push_all(&mut ime, "kanko").unwrap(), "かんこ");
        assert_eq!(ime.pending(), "");
    }

    #[test]
    fn double_n_commits_single_n_without_leftover() {
        let mut ime = RomajiKanaInput::new();

        // `nn` finishes ん and leaves nothing pending.
        assert_eq!(ime.push('n').unwrap().committed_text(), None);
        assert_eq!(ime.push('n').unwrap().committed_text(), Some("ん"));
        assert_eq!(ime.pending(), "");

        // な-row still needs an explicit vowel: `konnni` -> こんに.
        let mut ime = RomajiKanaInput::new();
        assert_eq!(push_all(&mut ime, "konnni").unwrap(), "こんに");
        assert_eq!(ime.pending(), "");
    }

    #[test]
    fn supports_sokuon_before_repeated_consonant() {
        let mut ime = RomajiKanaInput::new();

        assert_eq!(push_all(&mut ime, "kko").unwrap(), "っこ");
        assert_eq!(ime.pending(), "");
    }

    #[test]
    fn reset_cancels_pending_composition() {
        let mut ime = RomajiKanaInput::new();

        assert_eq!(ime.push('k').unwrap().committed_text(), None);
        assert!(ime.is_composing());
        ime.reset();
        assert_eq!(ime.pending(), "");
        assert_eq!(ime.finish().unwrap().committed_text(), None);
    }

    #[test]
    fn reports_incomplete_and_invalid_sequences() {
        let mut ime = RomajiKanaInput::new();

        assert_eq!(ime.push('k').unwrap().committed_text(), None);
        assert_eq!(ime.finish(), Err(ImeError::IncompleteComposition));
        assert_eq!(ime.pending(), "k");

        // `kw` is a valid incomplete prefix for the foreign-syllable forms
        // kwa/kwi/kwe/kwo, so keep it buffered and prove the completed path.
        assert_eq!(ime.push('w').unwrap().committed_text(), None);
        assert_eq!(ime.pending(), "kw");
        assert_eq!(ime.finish(), Err(ImeError::IncompleteComposition));
        assert_eq!(ime.push('a').unwrap().committed_text(), Some("くぁ"));
        assert_eq!(ime.pending(), "");

        assert_eq!(ime.push('k').unwrap().committed_text(), None);
        assert_eq!(ime.push('q'), Err(ImeError::InvalidSequence));
        assert_eq!(ime.pending(), "k");
        ime.reset();
        assert_eq!(ime.push('1'), Err(ImeError::UnsupportedInput));
    }

    #[test]
    fn sticky_shift_arms_next_character_only() {
        let mut shift = StickyShift::new();

        assert_eq!(
            shift.process(StickyShiftKey::Shift),
            StickyShiftOutput::None
        );
        assert!(shift.is_armed());
        assert_eq!(
            shift.process(StickyShiftKey::Character('k')),
            StickyShiftOutput::Character('K')
        );
        assert!(!shift.is_armed());
        assert_eq!(
            shift.process(StickyShiftKey::Character('a')),
            StickyShiftOutput::Character('a')
        );
    }

    #[test]
    fn sticky_shift_clears_after_one_non_shift_key() {
        let mut shift = StickyShift::new();

        shift.press_shift();
        assert_eq!(
            shift.process(StickyShiftKey::Other),
            StickyShiftOutput::None
        );
        assert!(!shift.is_armed());
        assert_eq!(
            shift.process(StickyShiftKey::Character('s')),
            StickyShiftOutput::Character('s')
        );
    }

    #[test]
    fn sticky_shift_repeated_shift_stays_armed_until_cancelled_or_used() {
        let mut shift = StickyShift::new();

        shift.process(StickyShiftKey::Shift);
        shift.process(StickyShiftKey::Shift);
        assert!(shift.is_armed());
        assert_eq!(
            shift.process(StickyShiftKey::Character('n')),
            StickyShiftOutput::Character('N')
        );
        assert!(!shift.is_armed());

        shift.process(StickyShiftKey::Shift);
        assert!(shift.is_armed());
        assert_eq!(
            shift.process(StickyShiftKey::Cancel),
            StickyShiftOutput::None
        );
        assert!(!shift.is_armed());
    }
}
