//! RGB565 → RGB666 pixel-format conversion for the ILI9488 present path
//! (KOTO-0174 H-D).
//!
//! The panel's reliable SPI format is 18-bit RGB666 (3 bytes/px), so every
//! presented pixel passes through this conversion on the CPU — and after the
//! H-A2 pipeline hid the SPI DMA, the convert is one of the three CPU terms
//! (vm / raster / convert) that set the frame wall. It lives here rather than
//! in the firmware's `lcd.rs` so the byte-exactness proof is host-testable
//! (koto-pico cannot run host tests — the `DiagProfile` precedent).
//!
//! The H-D optimization is pure byte algebra: for a little-endian RGB565 pixel
//! `lo, hi` (`value = hi<<8 | lo`), the three RGB666 bytes decompose into
//! single-byte operations —
//!
//! - `R = ((value >> 11) & 0x1f) << 3` = `hi & 0xf8`
//! - `G = ((value >> 5) & 0x3f) << 2` = `((hi & 0x07) << 5) | ((lo >> 5) << 2)`
//! - `B = (value & 0x1f) << 3`  = `lo << 3` (the `u8` shift drops bits 5..7)
//!
//! — so the u16 reassembly and wide shifts of the original loop vanish, and a
//! two-pixel unroll halves the iterator overhead. A lookup table was
//! considered and rejected: on the Cortex-M0+ an SRAM table load costs no less
//! than the one-cycle AND/shift it would replace.

/// Convert `width * height` little-endian RGB565 pixels (`pixels[..w*h*2]`)
/// into the ILI9488's RGB666 byte stream (`scratch[..w*h*3]`), returning the
/// RGB666 byte length or `None` if either buffer is too small. Byte-identical
/// to the pre-H-D reference loop (proven exhaustively in the tests below).
/// Pure and `no_std`. `#[inline]` because the firmware builds without LTO and
/// this was a same-crate `lcd.rs` function before the move — the crate
/// boundary must not cost what the rewrite saves.
#[inline]
pub fn convert_rgb565_to_rgb666(
    pixels: &[u8],
    scratch: &mut [u8],
    width: u16,
    height: u16,
) -> Option<usize> {
    let cells = width as usize * height as usize;
    let source_bytes = cells * 2;
    let target_bytes = cells * 3;
    if pixels.len() < source_bytes || scratch.len() < target_bytes {
        return None;
    }
    let pairs = cells / 2;
    let (src_pairs, src_tail) = pixels[..source_bytes].split_at(pairs * 4);
    let (dst_pairs, dst_tail) = scratch[..target_bytes].split_at_mut(pairs * 6);
    for (s, d) in src_pairs.chunks_exact(4).zip(dst_pairs.chunks_exact_mut(6)) {
        let lo = s[0];
        let hi = s[1];
        d[0] = hi & 0xf8;
        d[1] = ((hi & 0x07) << 5) | ((lo >> 5) << 2);
        d[2] = lo << 3;
        let lo = s[2];
        let hi = s[3];
        d[3] = hi & 0xf8;
        d[4] = ((hi & 0x07) << 5) | ((lo >> 5) << 2);
        d[5] = lo << 3;
    }
    if cells % 2 == 1 {
        let lo = src_tail[0];
        let hi = src_tail[1];
        dst_tail[0] = hi & 0xf8;
        dst_tail[1] = ((hi & 0x07) << 5) | ((lo >> 5) << 2);
        dst_tail[2] = lo << 3;
    }
    Some(target_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-H-D loop, kept verbatim as the reference the optimized path
    /// must match byte for byte.
    fn reference(pixels: &[u8], scratch: &mut [u8], width: u16, height: u16) -> Option<usize> {
        let source_bytes = width as usize * height as usize * 2;
        let target_bytes = width as usize * height as usize * 3;
        if pixels.len() < source_bytes || scratch.len() < target_bytes {
            return None;
        }
        for (rgb565, rgb666) in pixels[..source_bytes]
            .chunks_exact(2)
            .zip(scratch[..target_bytes].chunks_exact_mut(3))
        {
            let value = u16::from_le_bytes([rgb565[0], rgb565[1]]);
            rgb666[0] = ((value >> 11) as u8 & 0x1f) << 3;
            rgb666[1] = ((value >> 5) as u8 & 0x3f) << 2;
            rgb666[2] = (value as u8 & 0x1f) << 3;
        }
        Some(target_bytes)
    }

    /// Every possible RGB565 value, as one 256x256 surface: the optimized
    /// convert must reproduce the reference exactly.
    #[test]
    fn matches_reference_for_every_rgb565_value() {
        let mut pixels = vec![0u8; 65_536 * 2];
        for value in 0..=u16::MAX {
            let index = value as usize * 2;
            pixels[index..index + 2].copy_from_slice(&value.to_le_bytes());
        }
        let mut expected = vec![0u8; 65_536 * 3];
        let mut actual = vec![0u8; 65_536 * 3];
        assert_eq!(
            reference(&pixels, &mut expected, 256, 256),
            Some(65_536 * 3)
        );
        assert_eq!(
            convert_rgb565_to_rgb666(&pixels, &mut actual, 256, 256),
            Some(65_536 * 3)
        );
        assert_eq!(actual, expected);
    }

    /// Odd pixel counts exercise the unroll's tail pixel.
    #[test]
    fn matches_reference_for_odd_pixel_counts() {
        for cells in [1u16, 3, 5, 319] {
            let pixels: Vec<u8> = (0..cells as usize * 2)
                .map(|i| (i * 37 + 11) as u8)
                .collect();
            let mut expected = vec![0u8; cells as usize * 3];
            let mut actual = vec![0u8; cells as usize * 3];
            assert_eq!(
                reference(&pixels, &mut expected, cells, 1),
                Some(cells as usize * 3)
            );
            assert_eq!(
                convert_rgb565_to_rgb666(&pixels, &mut actual, cells, 1),
                Some(cells as usize * 3)
            );
            assert_eq!(actual, expected);
        }
    }

    /// Undersized buffers are rejected on either side, exactly as before.
    #[test]
    fn rejects_undersized_buffers() {
        let pixels = [0u8; 2 * 4];
        let mut scratch = [0u8; 3 * 4];
        assert_eq!(
            convert_rgb565_to_rgb666(&pixels[..6], &mut scratch, 2, 2),
            None
        );
        assert_eq!(
            convert_rgb565_to_rgb666(&pixels, &mut scratch[..11], 2, 2),
            None
        );
        assert_eq!(
            convert_rgb565_to_rgb666(&pixels, &mut scratch, 2, 2),
            Some(12)
        );
    }
}
