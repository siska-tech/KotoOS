//! Word packing helpers for QPI payload stream PIO programs.

/// Number of payload bytes in one 32-bit stream word.
pub(crate) const BYTES_PER_WORD: usize = 4;

/// Returns the exact QPI nibble count for a byte payload.
#[inline]
pub(crate) const fn nibble_count(byte_len: usize) -> usize {
    byte_len * 2
}

/// Returns the number of RX FIFO words expected from the read stream program.
///
/// The current read stream program performs one final unconditional `push` so
/// partial tail bytes are observable. Full-word reads therefore produce one
/// extra completion word that callers must drain and ignore.
#[inline]
pub(crate) const fn read_stream_rx_words(byte_len: usize) -> usize {
    (byte_len / BYTES_PER_WORD) + 1
}

/// Packs up to four bytes in the same MSB-first order used by the QPI command
/// byte path.
#[inline]
pub(crate) fn pack_stream_word(bytes: &[u8]) -> u32 {
    let mut packed = [0; BYTES_PER_WORD];
    let len = if bytes.len() > BYTES_PER_WORD {
        BYTES_PER_WORD
    } else {
        bytes.len()
    };
    packed[..len].copy_from_slice(&bytes[..len]);
    u32::from_be_bytes(packed)
}

/// Unpacks a stream RX word into up to four output bytes.
#[inline]
#[allow(dead_code)]
pub(crate) fn unpack_stream_word(word: u32, out: &mut [u8]) {
    let len = if out.len() > BYTES_PER_WORD {
        BYTES_PER_WORD
    } else {
        out.len()
    };

    match len {
        0 => {}
        1 => out[0] = (word >> 24) as u8,
        2 => {
            out[0] = (word >> 24) as u8;
            out[1] = (word >> 16) as u8;
        }
        3 => {
            out[0] = (word >> 24) as u8;
            out[1] = (word >> 16) as u8;
            out[2] = (word >> 8) as u8;
        }
        _ => unpack_stream_word_full(word, (&mut out[..BYTES_PER_WORD]).try_into().unwrap()),
    }
}

/// Unpacks one complete stream RX word into exactly four output bytes.
#[inline]
#[allow(dead_code)]
pub(crate) fn unpack_stream_word_full(word: u32, out: &mut [u8; BYTES_PER_WORD]) {
    // `word` is in stream byte order. Writing the big-endian numeric value with
    // an unaligned native store lays out the desired bytes on little-endian RP2040.
    unsafe {
        unpack_stream_word_full_unchecked(word, out.as_mut_ptr());
    }
}

/// Unpacks one complete stream RX word into exactly four bytes at `out`.
///
/// # Safety
///
/// `out` must be valid for writes of [`BYTES_PER_WORD`] bytes.
#[inline]
pub(crate) unsafe fn unpack_stream_word_full_unchecked(word: u32, out: *mut u8) {
    unsafe {
        core::ptr::write_unaligned(out.cast::<u32>(), word.to_be());
    }
}

/// Unpacks a final partial stream RX word.
#[inline]
pub(crate) fn unpack_stream_word_tail(word: u32, out: &mut [u8]) {
    debug_assert!(out.len() < BYTES_PER_WORD);
    match out.len() {
        0 => {}
        1 => out[0] = (word >> 24) as u8,
        2 => {
            out[0] = (word >> 24) as u8;
            out[1] = (word >> 16) as u8;
        }
        3 => {
            out[0] = (word >> 24) as u8;
            out[1] = (word >> 16) as u8;
            out[2] = (word >> 8) as u8;
        }
        _ => unreachable!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibble_count_is_exactly_two_per_byte() {
        assert_eq!(nibble_count(0), 0);
        assert_eq!(nibble_count(1), 2);
        assert_eq!(nibble_count(4), 8);
        assert_eq!(nibble_count(257), 514);
    }

    #[test]
    fn read_stream_rx_words_include_completion_word() {
        assert_eq!(read_stream_rx_words(0), 1);
        assert_eq!(read_stream_rx_words(1), 1);
        assert_eq!(read_stream_rx_words(3), 1);
        assert_eq!(read_stream_rx_words(4), 2);
        assert_eq!(read_stream_rx_words(5), 2);
        assert_eq!(read_stream_rx_words(8), 3);
    }

    #[test]
    fn pack_stream_word_preserves_command_path_byte_order() {
        assert_eq!(pack_stream_word(&[0x12]), 0x1200_0000);
        assert_eq!(pack_stream_word(&[0x12, 0x34]), 0x1234_0000);
        assert_eq!(pack_stream_word(&[0x12, 0x34, 0x56]), 0x1234_5600);
        assert_eq!(pack_stream_word(&[0x12, 0x34, 0x56, 0x78]), 0x1234_5678);
    }

    #[test]
    fn unpack_stream_word_respects_tail_lengths() {
        let word = 0x1234_5678;
        let mut out = [0; 4];

        unpack_stream_word(word, &mut out[..1]);
        assert_eq!(&out[..1], &[0x12]);

        unpack_stream_word(word, &mut out[..2]);
        assert_eq!(&out[..2], &[0x12, 0x34]);

        unpack_stream_word(word, &mut out[..3]);
        assert_eq!(&out[..3], &[0x12, 0x34, 0x56]);

        unpack_stream_word(word, &mut out);
        assert_eq!(out, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn unpack_stream_word_full_preserves_byte_order() {
        let mut out = [0; 4];
        unpack_stream_word_full(0x1234_5678, &mut out);
        assert_eq!(out, [0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn unpack_stream_word_tail_preserves_byte_order() {
        let word = 0x89ab_cdef;
        let mut out = [0; 4];

        unpack_stream_word_tail(word, &mut out[..1]);
        assert_eq!(&out[..1], &[0x89]);

        unpack_stream_word_tail(word, &mut out[..2]);
        assert_eq!(&out[..2], &[0x89, 0xab]);

        unpack_stream_word_tail(word, &mut out[..3]);
        assert_eq!(&out[..3], &[0x89, 0xab, 0xcd]);
    }

    #[test]
    fn unpack_stream_word_full_handles_alignment_boundaries() {
        for start in 0..BYTES_PER_WORD {
            let mut out = [0xee; 12];
            let chunk: &mut [u8; BYTES_PER_WORD] = (&mut out[start..start + BYTES_PER_WORD])
                .try_into()
                .unwrap();

            unpack_stream_word_full(0x0123_4567, chunk);

            assert_eq!(
                &out[start..start + BYTES_PER_WORD],
                &[0x01, 0x23, 0x45, 0x67]
            );
            assert!(out[..start].iter().all(|&byte| byte == 0xee));
            assert!(out[start + BYTES_PER_WORD..]
                .iter()
                .all(|&byte| byte == 0xee));
        }
    }

    #[test]
    fn pack_unpack_round_trips_acceptance_tail_lengths() {
        const LENGTHS: [usize; 12] = [1, 2, 3, 4, 5, 31, 32, 255, 256, 257, 512, 1023];
        let mut source = [0; 1023];
        for (index, byte) in source.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_mul(17).wrapping_add(0x23);
        }

        for len in LENGTHS {
            let mut out = [0; 1023];
            let mut offset = 0;
            while offset < len {
                let end = (offset + BYTES_PER_WORD).min(len);
                let word = pack_stream_word(&source[offset..end]);
                unpack_stream_word(word, &mut out[offset..end]);
                offset = end;
            }
            assert_eq!(&out[..len], &source[..len]);
        }
    }
}
