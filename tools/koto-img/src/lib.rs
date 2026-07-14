//! koto-img: PNG <-> `.kspr` sprite-sheet converter (KOTO-0187).
//!
//! `.kspr` is the committed, reviewable ASCII sprite source compiled to a
//! `KIM1` RGB565 strip by `harness/build_apps.py`. This crate adds the paths
//! into and out of those formats so art can be authored in any paint tool:
//!
//! - PNG -> `.kspr`: slice a PNG (width and height multiples of 16) into
//!   16x16 tiles, left-to-right then top-to-bottom, and emit deterministic
//!   `.kspr` text whose palette is built from the exact colors present.
//! - `.kspr` / `KIM1` -> PNG: render what the device renders. Palette colors
//!   are truncated to RGB565 exactly as `build_apps.py` packs them, then
//!   expanded back to 8-bit by bit replication. Feeding that PNG back through
//!   PNG -> `.kspr` therefore reproduces a byte-identical `.kim`.
//!
//! The RGB565 truncation is the one lossy step: `r & 0xF8`, `g & 0xFC`,
//! `b & 0xF8` survive, low bits are dropped. Expansion replicates the high
//! bits into the low bits (`r | r >> 5`, `g | g >> 6`, `b | b >> 5`), which
//! is idempotent, so 565-representable inputs round-trip exactly.

use std::collections::HashMap;
use std::fmt::Write as _;

/// Tile edge length in pixels; `.kspr` tiles are exactly 16x16.
pub const TILE: usize = 16;

/// Palette characters in assignment order for PNG -> `.kspr`.
///
/// Excludes `#` (starts a comment line) and whitespace (row lines are exactly
/// 16 palette characters). `.` is first by convention: existing sheets use it
/// as the background color.
pub const PALETTE_ALPHABET: &str =
    ".,:;-=+*oxOX%&@abcdefghijklmnpqrstuvwyz0123456789ABCDEFGHIJKLMNPQ";

/// Pack 8-bit RGB into RGB565, exactly as `build_apps.py::rgb565_le` does.
pub fn rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 & 0xF8) << 8) | ((g as u16 & 0xFC) << 3) | (b as u16 >> 3)
}

/// Expand RGB565 back to 8-bit RGB by bit replication.
pub fn expand565(p: u16) -> [u8; 3] {
    let r = ((p >> 8) & 0xF8) as u8;
    let g = ((p >> 3) & 0xFC) as u8;
    let b = ((p << 3) & 0xF8) as u8;
    [r | (r >> 5), g | (g >> 6), b | (b >> 5)]
}

/// Compile `.kspr` text into `KIM1` bytes.
///
/// Mirrors `harness/build_apps.py::kspr_to_kim`: `# comment` and blank lines
/// are skipped, `color <char> <RRGGBB>` defines a palette entry, `tile <id>
/// <name>` starts a tile, and every other line is a 16-character pixel row.
/// Output is `b"KIM1"`, width/height as little-endian u16, then row-major
/// little-endian RGB565.
pub fn compile_kspr(text: &str) -> Result<Vec<u8>, String> {
    let mut palette: HashMap<char, [u8; 3]> = HashMap::new();
    let mut tiles: Vec<Vec<Vec<char>>> = Vec::new();
    for raw in text.lines() {
        let s = raw.trim();
        if s.is_empty() || s.starts_with('#') {
            continue;
        }
        if let Some(rest) = s.strip_prefix("color ") {
            let parts: Vec<&str> = rest.split_whitespace().collect();
            let (ch, hex) = match parts.as_slice() {
                [ch, hex] if ch.chars().count() == 1 && hex.len() == 6 => {
                    (ch.chars().next().unwrap(), *hex)
                }
                _ => return Err(format!("bad palette line: {s:?}")),
            };
            let rgb = parse_hex6(hex).ok_or_else(|| format!("bad palette line: {s:?}"))?;
            palette.insert(ch, rgb);
        } else if s.starts_with("tile ") {
            tiles.push(Vec::new());
        } else {
            let Some(tile) = tiles.last_mut() else {
                return Err(format!("pixel row before any `tile`: {s:?}"));
            };
            let row: Vec<char> = s.chars().collect();
            if row.len() != TILE {
                return Err(format!(
                    "row is {} chars, expected {TILE}: {s:?}",
                    row.len()
                ));
            }
            tile.push(row);
        }
    }
    if tiles.is_empty() {
        return Err("no tiles defined".to_string());
    }
    let mut out = Vec::with_capacity(8 + tiles.len() * TILE * TILE * 2);
    out.extend_from_slice(b"KIM1");
    out.extend_from_slice(&(TILE as u16).to_le_bytes());
    out.extend_from_slice(&((TILE * tiles.len()) as u16).to_le_bytes());
    for (index, rows) in tiles.iter().enumerate() {
        if rows.len() != TILE {
            return Err(format!(
                "tile {index} has {} rows, expected {TILE}",
                rows.len()
            ));
        }
        for row in rows {
            for &ch in row {
                let [r, g, b] = *palette
                    .get(&ch)
                    .ok_or_else(|| format!("undefined palette char {ch:?}"))?;
                out.extend_from_slice(&rgb565(r, g, b).to_le_bytes());
            }
        }
    }
    Ok(out)
}

fn parse_hex6(hex: &str) -> Option<[u8; 3]> {
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

/// Decode `KIM1` bytes into `(width, height, row-major RGB8)`.
///
/// Pixels are expanded from RGB565 by bit replication ([`expand565`]), so the
/// PNG shows exactly what the device blits.
pub fn kim_to_rgb8(kim: &[u8]) -> Result<(u16, u16, Vec<u8>), String> {
    if kim.len() < 8 || &kim[0..4] != b"KIM1" {
        return Err("not a KIM1 image (bad magic)".to_string());
    }
    let width = u16::from_le_bytes([kim[4], kim[5]]);
    let height = u16::from_le_bytes([kim[6], kim[7]]);
    let expected = 8 + width as usize * height as usize * 2;
    if kim.len() != expected {
        return Err(format!(
            "KIM1 payload is {} bytes, expected {expected} for {width}x{height}",
            kim.len()
        ));
    }
    let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
    for chunk in kim[8..].chunks_exact(2) {
        rgb.extend_from_slice(&expand565(u16::from_le_bytes([chunk[0], chunk[1]])));
    }
    Ok((width, height, rgb))
}

/// Build `.kspr` text from decoded RGBA8 pixels.
///
/// Requirements: width and height are non-zero multiples of 16 and every
/// pixel is fully opaque (`.kspr`/`draw_pixels` has no transparency). Tiles
/// are sliced left-to-right, top-to-bottom; palette characters are assigned
/// in first-appearance order over the emitted tile pixels, so the palette
/// reads in the same order as the tile rows below it. Colors are kept exact:
/// a PNG with more distinct colors than the palette alphabet is rejected, not
/// quantized (the RGB565 truncation happens at `.kim` build time, same as for
/// hand-written sheets).
pub fn rgba_to_kspr(width: u32, height: u32, rgba: &[u8]) -> Result<String, String> {
    if width == 0
        || height == 0
        || !width.is_multiple_of(TILE as u32)
        || !height.is_multiple_of(TILE as u32)
    {
        return Err(format!(
            "image is {width}x{height}; both sides must be non-zero multiples of {TILE}"
        ));
    }
    if rgba.len() != width as usize * height as usize * 4 {
        return Err("pixel buffer size does not match dimensions".to_string());
    }
    let opaque_violations = rgba.chunks_exact(4).filter(|px| px[3] != 255).count();
    if opaque_violations > 0 {
        return Err(format!(
            "{opaque_violations} pixel(s) are not fully opaque; .kspr has no transparency"
        ));
    }
    let cols = width as usize / TILE;
    let rows = height as usize / TILE;
    let alphabet: Vec<char> = PALETTE_ALPHABET.chars().collect();
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut index_of: HashMap<[u8; 3], usize> = HashMap::new();
    let mut tiles: Vec<Vec<usize>> = Vec::new();
    for ty in 0..rows {
        for tx in 0..cols {
            let mut tile = Vec::with_capacity(TILE * TILE);
            for py in 0..TILE {
                for px in 0..TILE {
                    let o = ((ty * TILE + py) * width as usize + tx * TILE + px) * 4;
                    let rgb = [rgba[o], rgba[o + 1], rgba[o + 2]];
                    let idx = *index_of.entry(rgb).or_insert_with(|| {
                        palette.push(rgb);
                        palette.len() - 1
                    });
                    tile.push(idx);
                }
            }
            tiles.push(tile);
        }
    }
    if palette.len() > alphabet.len() {
        return Err(format!(
            "image has {} distinct colors; the .kspr palette holds at most {}",
            palette.len(),
            alphabet.len()
        ));
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "# koto-img png2kspr — {} tiles, {} colors",
        tiles.len(),
        palette.len()
    );
    for (i, [r, g, b]) in palette.iter().enumerate() {
        let _ = writeln!(out, "color {} {r:02X}{g:02X}{b:02X}", alphabet[i]);
    }
    for (i, tile) in tiles.iter().enumerate() {
        let _ = writeln!(out, "\ntile {i} t{i}");
        for row in tile.chunks_exact(TILE) {
            for &idx in row {
                out.push(alphabet[idx]);
            }
            out.push('\n');
        }
    }
    Ok(out)
}

/// Decode a PNG into `(width, height, row-major RGBA8)`.
///
/// Any bit depth / color type a paint tool exports (palette, grayscale, with
/// or without alpha) is normalized to 8-bit RGBA.
pub fn decode_png_rgba(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| format!("png: {e}"))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("png: {e}"))?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .flat_map(|px| [px[0], px[1], px[2], 255])
            .collect(),
        png::ColorType::Grayscale => buf.iter().flat_map(|&v| [v, v, v, 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .flat_map(|px| [px[0], px[0], px[0], px[1]])
            .collect(),
        other => return Err(format!("png: unsupported color type {other:?}")),
    };
    Ok((info.width, info.height, rgba))
}

/// Encode row-major RGB8 pixels as a PNG.
pub fn encode_png_rgb(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, String> {
    if rgb.len() != width as usize * height as usize * 3 {
        return Err("pixel buffer size does not match dimensions".to_string());
    }
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| format!("png: {e}"))?;
    writer
        .write_image_data(rgb)
        .map_err(|e| format!("png: {e}"))?;
    writer.finish().map_err(|e| format!("png: {e}"))?;
    Ok(out)
}

/// Launcher-icon edge length; `.kicon` (`KICON1`) is a 40x40 1-bit mask.
pub const KICON_SIZE: usize = 40;

/// Parse `KICON1` text into a 40x40 mask grid (`true` = set pixel, `#`).
///
/// The format is the `KICON1` magic line followed by exactly [`KICON_SIZE`]
/// rows of [`KICON_SIZE`] `#`/`.` characters (see `asset_pipeline.py`).
pub fn parse_kicon(text: &str) -> Result<Vec<Vec<bool>>, String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("KICON1") {
        return Err("not a KICON1 icon (missing magic)".to_string());
    }
    let mut grid = Vec::with_capacity(KICON_SIZE);
    for line in lines {
        if let Some(bad) = line.chars().find(|&c| c != '#' && c != '.') {
            return Err(format!("row {}: unexpected char {bad:?}", grid.len() + 1));
        }
        let row: Vec<bool> = line.chars().map(|c| c == '#').collect();
        if row.len() != KICON_SIZE {
            return Err(format!(
                "row {} is {} chars, expected {KICON_SIZE}",
                grid.len() + 1,
                row.len()
            ));
        }
        grid.push(row);
    }
    if grid.len() != KICON_SIZE {
        return Err(format!("{} rows, expected {KICON_SIZE}", grid.len()));
    }
    Ok(grid)
}

/// Render a mask grid as row-major RGB8: set pixels black, unset white. This
/// is the neutral converter preview; the editor recolors through the app's
/// `shell_icon` palette.
pub fn kicon_to_rgb(grid: &[Vec<bool>]) -> (u32, u32, Vec<u8>) {
    let mut rgb = Vec::with_capacity(KICON_SIZE * KICON_SIZE * 3);
    for row in grid {
        for &set in row {
            let v = if set { 0 } else { 255 };
            rgb.extend_from_slice(&[v, v, v]);
        }
    }
    (KICON_SIZE as u32, KICON_SIZE as u32, rgb)
}

/// Build `KICON1` text from decoded RGBA8 pixels (40x40 required).
///
/// A pixel is set (`#`) when its luminance composited over white is dark
/// (< 50%). That handles both dark-on-white and dark-on-transparent art from
/// paint tools, and round-trips [`kicon_to_rgb`]'s black/white output exactly.
pub fn rgba_to_kicon(width: u32, height: u32, rgba: &[u8]) -> Result<String, String> {
    if width as usize != KICON_SIZE || height as usize != KICON_SIZE {
        return Err(format!(
            "icon is {width}x{height}; must be {KICON_SIZE}x{KICON_SIZE}"
        ));
    }
    if rgba.len() != KICON_SIZE * KICON_SIZE * 4 {
        return Err("pixel buffer size does not match dimensions".to_string());
    }
    let mut out = String::with_capacity(7 + KICON_SIZE * (KICON_SIZE + 1));
    out.push_str("KICON1\n");
    for row in rgba.chunks_exact(KICON_SIZE * 4) {
        for px in row.chunks_exact(4) {
            let lum =
                0.299 * f32::from(px[0]) + 0.587 * f32::from(px[1]) + 0.114 * f32::from(px[2]);
            let alpha = f32::from(px[3]) / 255.0;
            let over_white = alpha * lum + (1.0 - alpha) * 255.0;
            out.push(if over_white < 128.0 { '#' } else { '.' });
        }
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo_path(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(rel)
    }

    /// Compiling the shipped KotoRogue sheet must reproduce the committed
    /// `.kim` byte-for-byte — the Rust compiler mirrors `build_apps.py`.
    #[test]
    fn compile_matches_build_apps_output_for_kotorogue() {
        let kspr = std::fs::read_to_string(repo_path("apps/kotorogue/sprites/tiles.kspr"))
            .expect("read tiles.kspr");
        let committed = std::fs::read(repo_path("package_inputs/sprites/kotorogue_tiles.kim"))
            .expect("read committed kim");
        assert_eq!(compile_kspr(&kspr).expect("compile"), committed);
    }

    /// The proving case for KOTO-0187: export the shipped sheet to PNG pixels
    /// (through the real PNG codec), re-import, recompile — the `.kim` is
    /// byte-identical even though palette characters and names differ.
    #[test]
    fn kotorogue_png_round_trip_reproduces_identical_kim() {
        let kspr = std::fs::read_to_string(repo_path("apps/kotorogue/sprites/tiles.kspr"))
            .expect("read tiles.kspr");
        let kim = compile_kspr(&kspr).expect("compile");
        let (w, h, rgb) = kim_to_rgb8(&kim).expect("decode kim");
        let png = encode_png_rgb(w as u32, h as u32, &rgb).expect("encode png");
        let (pw, ph, rgba) = decode_png_rgba(&png).expect("decode png");
        assert_eq!((pw, ph), (w as u32, h as u32));
        let reimported = rgba_to_kspr(pw, ph, &rgba).expect("to kspr");
        assert_eq!(compile_kspr(&reimported).expect("recompile"), kim);
    }

    /// KOTO-0196 proving case: export a shipped `.kicon` to PNG (through the
    /// real codec) and re-import — the mask text is reproduced byte-identically
    /// (modulo the source file's line endings).
    #[test]
    fn kotorogue_icon_png_round_trip_reproduces_identical_kicon() {
        let raw = std::fs::read_to_string(repo_path("apps/kotorogue/icon.kicon"))
            .expect("read icon.kicon");
        let normalized = raw.replace("\r\n", "\n");
        let grid = parse_kicon(&normalized).expect("parse kicon");
        let (w, h, rgb) = kicon_to_rgb(&grid);
        let png = encode_png_rgb(w, h, &rgb).expect("encode png");
        let (pw, ph, rgba) = decode_png_rgba(&png).expect("decode png");
        let reimported = rgba_to_kicon(pw, ph, &rgba).expect("to kicon");
        assert_eq!(reimported, normalized);
    }

    #[test]
    fn parse_kicon_rejects_malformed_masks() {
        assert!(parse_kicon("nope\n").unwrap_err().contains("missing magic"));
        let short = format!("KICON1\n{}", format!("{}\n", "#".repeat(40)).repeat(39));
        assert!(parse_kicon(&short).unwrap_err().contains("39 rows"));
        let bad_char = format!("KICON1\n{}\n", "x".repeat(40));
        assert!(parse_kicon(&bad_char)
            .unwrap_err()
            .contains("unexpected char"));
        let narrow = format!("KICON1\n{}", format!("{}\n", "#".repeat(20)).repeat(40));
        assert!(parse_kicon(&narrow).unwrap_err().contains("20 chars"));
    }

    #[test]
    fn rgba_to_kicon_thresholds_and_requires_40x40() {
        let opaque_black = vec![0, 0, 0, 255];
        let white = vec![255, 255, 255, 255];
        let transparent = vec![0, 0, 0, 0];
        let mut px = Vec::new();
        px.extend(&opaque_black);
        px.extend(&white);
        px.extend(&transparent);
        px.extend(vec![0u8; (KICON_SIZE * KICON_SIZE - 3) * 4]);
        let text = rgba_to_kicon(KICON_SIZE as u32, KICON_SIZE as u32, &px).expect("to kicon");
        let first_row = text.lines().nth(1).unwrap();
        assert_eq!(&first_row[0..3], "#.."); // black set, white/transparent unset
        assert!(rgba_to_kicon(32, 40, &[])
            .unwrap_err()
            .contains("must be 40x40"));
    }

    /// Bit-replication expansion is idempotent: every 565-representable color
    /// survives a second truncate/expand cycle exactly.
    #[test]
    fn expand565_is_idempotent_over_all_pixels() {
        for p in 0..=u16::MAX {
            let [r, g, b] = expand565(p);
            assert_eq!(expand565(rgb565(r, g, b)), [r, g, b], "pixel {p:#06x}");
        }
    }

    #[test]
    fn rgba_to_kspr_slices_grids_left_to_right_then_top_to_bottom() {
        // 32x32 = 4 tiles, each a solid color; strip order must be
        // top-left, top-right, bottom-left, bottom-right.
        let colors = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
        let mut rgba = vec![0u8; 32 * 32 * 4];
        for y in 0..32 {
            for x in 0..32 {
                let c = colors[(y / 16) * 2 + x / 16];
                let o = (y * 32 + x) * 4;
                rgba[o..o + 3].copy_from_slice(&c);
                rgba[o + 3] = 255;
            }
        }
        let kspr = rgba_to_kspr(32, 32, &rgba).expect("to kspr");
        let kim = compile_kspr(&kspr).expect("compile");
        let (w, h, rgb) = kim_to_rgb8(&kim).expect("decode");
        assert_eq!((w, h), (16, 64));
        for (tile, c) in colors.iter().enumerate() {
            let o = tile * 16 * 16 * 3;
            assert_eq!(&rgb[o..o + 3], &expand565(rgb565(c[0], c[1], c[2]))[..]);
        }
    }

    #[test]
    fn rgba_to_kspr_rejects_bad_dimensions_alpha_and_color_budget() {
        let opaque = |n: usize| {
            let mut v = vec![0u8; n * 4];
            v.chunks_exact_mut(4).for_each(|px| px[3] = 255);
            v
        };
        assert!(rgba_to_kspr(15, 16, &opaque(15 * 16))
            .unwrap_err()
            .contains("multiples"));
        assert!(rgba_to_kspr(0, 16, &[]).unwrap_err().contains("multiples"));

        let mut translucent = opaque(16 * 16);
        translucent[3] = 128;
        translucent[7] = 0;
        let err = rgba_to_kspr(16, 16, &translucent).unwrap_err();
        assert!(err.contains("2 pixel(s)"), "{err}");

        // 16x16 = 256 distinct colors > alphabet budget.
        let mut many = opaque(16 * 16);
        for (i, px) in many.chunks_exact_mut(4).enumerate() {
            px[0] = i as u8;
            px[1] = (i >> 4) as u8;
        }
        let err = rgba_to_kspr(16, 16, &many).unwrap_err();
        assert!(err.contains("256 distinct colors"), "{err}");
    }

    #[test]
    fn compile_kspr_rejects_malformed_sources() {
        assert!(compile_kspr("").unwrap_err().contains("no tiles"));
        assert!(compile_kspr("................\n")
            .unwrap_err()
            .contains("before any"));
        assert!(compile_kspr("color ! GGGGGG\n")
            .unwrap_err()
            .contains("bad palette"));
        assert!(compile_kspr("color ! FFF\n")
            .unwrap_err()
            .contains("bad palette"));
        let short_row = "color . 000000\ntile 0 a\n........\n";
        assert!(compile_kspr(short_row).unwrap_err().contains("8 chars"));
        let undefined = format!(
            "color . 000000\ntile 0 a\n{}",
            format!("{}\n", "x".repeat(16)).repeat(16)
        );
        assert!(compile_kspr(&undefined)
            .unwrap_err()
            .contains("undefined palette char"));
        let missing_rows = "color . 000000\ntile 0 a\n................\n";
        assert!(compile_kspr(missing_rows).unwrap_err().contains("1 rows"));
    }

    #[test]
    fn palette_alphabet_is_unique_and_safe() {
        let chars: Vec<char> = PALETTE_ALPHABET.chars().collect();
        let mut seen = std::collections::HashSet::new();
        for &c in &chars {
            assert!(!c.is_whitespace() && c != '#', "unsafe palette char {c:?}");
            assert!(seen.insert(c), "duplicate palette char {c:?}");
        }
    }
}
