# KOTO-0100: Romaji-to-kana missing youon rows

- Status: done
- Type: bug
- Priority: P1
- Requirements: FR-IME-1

## Goal

Let users type every standard youon (拗音) syllable, so common words such as
きょう, ひゃく, and みょう compose instead of being rejected as invalid romaji.

## Acceptance Criteria

- [x] き / ひ / み youon rows (`kya/kyu/kyo`, `hya/hyu/hyo`, `mya/myu/myo`) compose to kana.
- [x] The previously working rows (sha, cha, nya, rya, …) still compose.
- [x] A regression test covers every standard consonant + ゃ/ゅ/ょ row.

## Notes

The `ROMAJI_KANA` table in [`src/koto-core/src/ime.rs`](../../../src/koto-core/src/ime.rs)
listed gya/sha/ja/cha/nya/bya/pya/rya youon but omitted the き, ひ, and み rows.
Typing e.g. `k` `y` left `ky`, which is not a prefix of any table key, so the
composer returned `InvalidSequence` and the keystrokes were dropped/recovered as
raw ASCII. にゃ/ちゃ/りゃ (named in the report) actually worked; the user hit the
missing rows in real words and generalized.

Fix: add the nine missing Hepburn youon entries (`kya/kyu/kyo`, `hya/hyu/hyo`,
`mya/myu/myo`). The composer logic already handles consonant+`y`+vowel via the
`is_consonant(ch) && ch != b'y'` carve-out for the syllabic ん, so only the table
needed completing.

Verification: `converts_all_standard_youon_rows` in
[`src/koto-core/src/ime.rs`](../../../src/koto-core/src/ime.rs) asserts every standard
youon row composes and leaves no pending bytes.

Possible follow-up (out of scope): kunrei-style alternates (`sya/tya/jya/zya …`)
and small-kana/foreign syllables (`fa/va/che/xtu …`).
