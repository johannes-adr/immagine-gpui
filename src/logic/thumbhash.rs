//! Owner: A5. Swift source: `../immagine/Immagine/UI/Timeline/ThumbhashDecoder.swift`.
//! Spec: `../immagine/ImmagineTests/` -> DisplayImageTests.
//! Port the tests first (same names in snake_case), then implement until `cargo test --lib logic::thumbhash` is green.
//!
//! Port of the reference ThumbHash decoder (evanw/thumbhash). A base64 thumbhash becomes a tiny
//! premultiplied RGBA bitmap: the tier-A placeholder that shows instantly, even offline.
//! There are no XCTests for the Swift decoder; the tests below check invariants and hand-built hashes.

/// Decodes a base64 thumbhash into `(rgba, width, height)`.
///
/// `rgba` is premultiplied, 4 bytes per pixel, row-major, the same bytes the Swift decoder put into
/// its premultiplied-last CGImage. `None` for invalid base64 or a truncated hash.
pub fn decode(hash: &str) -> Option<(Vec<u8>, u32, u32)> {
    let bytes = base64_decode(hash)?;
    if bytes.len() < 6 {
        return None;
    }

    // Header constants.
    let header24 = bytes[0] as usize | (bytes[1] as usize) << 8 | (bytes[2] as usize) << 16;
    let header16 = bytes[3] as usize | (bytes[4] as usize) << 8;
    let l_dc = (header24 & 63) as f64 / 63.0;
    let p_dc = ((header24 >> 6) & 63) as f64 / 31.5 - 1.0;
    let q_dc = ((header24 >> 12) & 63) as f64 / 31.5 - 1.0;
    let l_scale = ((header24 >> 18) & 31) as f64 / 31.0;
    let has_alpha = (header24 >> 23) != 0;
    let p_scale = ((header16 >> 3) & 63) as f64 / 63.0;
    let q_scale = ((header16 >> 9) & 63) as f64 / 63.0;
    let is_landscape = (header16 >> 15) != 0;
    let lx = 3.max(if is_landscape {
        if has_alpha { 5 } else { 7 }
    } else {
        header16 & 7
    });
    let ly = 3.max(if is_landscape {
        header16 & 7
    } else if has_alpha {
        5
    } else {
        7
    });
    let a_dc = if has_alpha { (bytes[5] & 15) as f64 / 15.0 } else { 1.0 };
    let a_scale = (bytes[5] >> 4) as f64 / 15.0;

    // AC nibbles needed: L has lx*ly-1, P and Q have 8 each, A (alpha) has 24.
    let nibbles = lx * ly - 1 + 16 + if has_alpha { 24 } else { 0 };
    let ac_start = if has_alpha { 6 } else { 5 };
    if bytes.len() < ac_start + (nibbles + 1) / 2 {
        return None; // truncated hash
    }
    let mut ac_index = 0;
    let l_ac = decode_channel(&bytes, ac_start, &mut ac_index, lx, ly, l_scale);
    let p_ac = decode_channel(&bytes, ac_start, &mut ac_index, 3, 3, p_scale * 1.25);
    let q_ac = decode_channel(&bytes, ac_start, &mut ac_index, 3, 3, q_scale * 1.25);
    let a_ac = if has_alpha { decode_channel(&bytes, ac_start, &mut ac_index, 5, 5, a_scale) } else { Vec::new() };

    // Output size from the approximate aspect ratio.
    let ratio = lx as f64 / ly as f64;
    let w = 1.max((if ratio > 1.0 { 32.0 } else { 32.0 * ratio }).round() as u32);
    let h = 1.max((if ratio > 1.0 { 32.0 / ratio } else { 32.0 }).round() as u32);

    // Decode with the DCT.
    let max_nx = lx.max(if has_alpha { 5 } else { 3 });
    let max_ny = ly.max(if has_alpha { 5 } else { 3 });
    let mut fx = vec![0.0; max_nx];
    let mut fy = vec![0.0; max_ny];
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    let mut i = 0;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let (mut l, mut p, mut q, mut a) = (l_dc, p_dc, q_dc, a_dc);
            for (cx, f) in fx.iter_mut().enumerate() {
                *f = (std::f64::consts::PI / w as f64 * (x as f64 + 0.5) * cx as f64).cos();
            }
            for (cy, f) in fy.iter_mut().enumerate() {
                *f = (std::f64::consts::PI / h as f64 * (y as f64 + 0.5) * cy as f64).cos();
            }

            let mut j = 0;
            for cy in 0..ly {
                let mut cx = if cy == 0 { 1 } else { 0 };
                let fy2 = fy[cy] * 2.0;
                while cx * ly < lx * (ly - cy) {
                    l += l_ac[j] * fx[cx] * fy2;
                    j += 1;
                    cx += 1;
                }
            }
            j = 0;
            for cy in 0..3 {
                let mut cx = if cy == 0 { 1 } else { 0 };
                let fy2 = fy[cy] * 2.0;
                while cx < 3 - cy {
                    let f = fx[cx] * fy2;
                    p += p_ac[j] * f;
                    q += q_ac[j] * f;
                    j += 1;
                    cx += 1;
                }
            }
            if has_alpha {
                j = 0;
                for cy in 0..5 {
                    let mut cx = if cy == 0 { 1 } else { 0 };
                    let fy2 = fy[cy] * 2.0;
                    while cx < 5 - cy {
                        a += a_ac[j] * fx[cx] * fy2;
                        j += 1;
                        cx += 1;
                    }
                }
            }

            let b = l - 2.0 / 3.0 * p;
            let r = (3.0 * l - b + q) / 2.0;
            let g = r - q;
            let alpha = a.clamp(0.0, 1.0);
            // Premultiplied RGBA (truncating, like the Uint8 assignment in the reference).
            rgba[i] = (r.clamp(0.0, 1.0) * alpha * 255.0) as u8;
            rgba[i + 1] = (g.clamp(0.0, 1.0) * alpha * 255.0) as u8;
            rgba[i + 2] = (b.clamp(0.0, 1.0) * alpha * 255.0) as u8;
            rgba[i + 3] = (alpha * 255.0) as u8;
            i += 4;
        }
    }

    Some((rgba, w, h))
}

/// Reads `nx * ny` AC coefficients (4-bit nibbles, low nibble first) starting at `*next`.
fn decode_channel(bytes: &[u8], ac_start: usize, next: &mut usize, nx: usize, ny: usize, scale: f64) -> Vec<f64> {
    let mut ac = Vec::with_capacity(nx * ny);
    for cy in 0..ny {
        let mut cx = if cy == 0 { 1 } else { 0 };
        while cx * ny < nx * (ny - cy) {
            let value = (bytes[ac_start + (*next >> 1)] as usize >> ((*next & 1) << 2)) & 15;
            *next += 1;
            ac.push((value as f64 / 7.5 - 1.0) * scale);
            cx += 1;
        }
    }
    ac
}

/// Standard base64 (RFC 4648 alphabet). Padding is optional, as the Swift code adds it before decoding.
/// `None` for characters outside the alphabet or an impossible length.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let trimmed = input.trim_end_matches('=');
    if trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for c in trimmed.bytes() {
        let value = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standard base64 with padding, to build hashes in the tests.
    fn base64_encode(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
            for k in 0..4 {
                if k <= chunk.len() {
                    out.push(ALPHABET[(n >> (18 - 6 * k) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    /// A hash without any AC terms (all scales zero): every pixel is the DC colour.
    /// `l6`, `p6`, `q6` are the 6-bit DC values; `landscape_or_size` goes into the low 3 bits of header16.
    fn flat_hash(l6: u32, p6: u32, q6: u32, size_bits: u8, landscape: bool, alpha: bool) -> String {
        let header24: u32 = l6 | p6 << 6 | q6 << 12 | (alpha as u32) << 23;
        let header16: u32 = size_bits as u32 | (landscape as u32) << 15;
        let mut bytes = vec![header24 as u8, (header24 >> 8) as u8, (header24 >> 16) as u8, header16 as u8, (header16 >> 8) as u8];
        if alpha {
            bytes.push(15); // alpha DC 1, alpha scale 0
        } else {
            bytes.push(0);
        }
        let lx = 3.max(if landscape { if alpha { 5 } else { 7 } } else { size_bits as usize });
        let ly = 3.max(if landscape { size_bits as usize } else if alpha { 5 } else { 7 });
        let nibbles = lx * ly - 1 + 16 + if alpha { 24 } else { 0 };
        let ac_start = if alpha { 6 } else { 5 };
        bytes.resize(ac_start + (nibbles + 1) / 2, 0);
        base64_encode(&bytes)
    }

    #[test]
    fn base64_decodes_rfc_vectors() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYg").unwrap(), b"foob", "unpadded input is accepted");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
    }

    #[test]
    fn base64_rejects_bad_characters_and_lengths() {
        assert_eq!(base64_decode("Zm9v!"), None);
        assert_eq!(base64_decode("Z"), None, "one leftover character is impossible");
        assert_eq!(base64_decode("Zm=9"), None, "padding in the middle");
    }

    #[test]
    fn empty_or_short_hash_gives_nil() {
        assert_eq!(decode(""), None);
        assert_eq!(decode(&base64_encode(&[1, 2, 3])), None, "fewer than six header bytes");
    }

    #[test]
    fn truncated_hash_gives_nil() {
        let full = flat_hash(63, 31, 31, 3, false, false);
        let bytes = base64_decode(&full).unwrap();
        assert_eq!(decode(&base64_encode(&bytes[..bytes.len() - 1])), None);
    }

    #[test]
    fn flat_hash_decodes_to_a_uniform_bitmap() {
        // L fully bright, P and Q near zero: an almost white image.
        let (rgba, w, h) = decode(&flat_hash(63, 31, 31, 3, false, false)).unwrap();
        assert_eq!((w, h), (14, 32), "portrait: lx 3, ly 7 -> 32 × 14");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        let first = &rgba[0..4];
        assert!((first[0] as i32 - 251).abs() <= 2, "red {}", first[0]);
        assert_eq!(first[1], 255);
        assert_eq!(first[2], 255);
        assert_eq!(first[3], 255, "opaque without alpha channel");
        assert!(rgba.chunks(4).all(|px| px == first), "no AC terms: every pixel is the same");
    }

    #[test]
    fn black_dc_gives_black_pixels() {
        let (rgba, _, _) = decode(&flat_hash(0, 31, 31, 3, false, false)).unwrap();
        assert!(rgba.chunks(4).all(|px| px[0] == 0 && px[1] == 0 && px[2] <= 3 && px[3] == 255), "black: {:?}", &rgba[0..4]);
    }

    #[test]
    fn landscape_hash_is_wide() {
        let (_, w, h) = decode(&flat_hash(63, 31, 31, 3, true, false)).unwrap();
        assert_eq!((w, h), (32, 14), "landscape: lx 7, ly 3");
    }

    #[test]
    fn alpha_hash_reads_the_alpha_channel_length() {
        // With alpha the header is 6 bytes; the decoder takes the alpha DC (15 -> 1.0) and the aspect from header16.
        let (rgba, w, h) = decode(&flat_hash(63, 31, 31, 3, false, true)).unwrap();
        assert_eq!((w, h), (19, 32), "alpha: lx 3, ly 5 -> 0.6 aspect");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        assert!(rgba.chunks(4).all(|px| px[3] == 255));
    }
}
