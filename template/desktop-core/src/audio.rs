//! Audio helpers shared by speech in and out: WAV read and write, 16-bit PCM, resampling,
//! and base64 for passing audio to and from the web view. Plain Rust, no dependencies.
//!
//! # Example
//!
//! ```
//! use desktop_core::audio::{base64, base64_decode, read_wav, resample, wav};
//!
//! let tone: Vec<f32> = (0..480).map(|i| (i as f32 * 0.1).sin() * 0.5).collect();
//! let bytes = wav(&tone, 48_000);
//! let (samples, rate) = read_wav(&bytes)?;
//! assert_eq!((samples.len(), rate), (480, 48_000));
//! assert_eq!(resample(&samples, 48_000, 16_000).len(), 160);
//! assert_eq!(base64_decode(&base64(&bytes))?, bytes);
//! # Ok::<(), desktop_core::Error>(())
//! ```

use crate::{Error, Result};

fn audio_err(e: impl std::fmt::Display) -> Error {
    Error::Source(format!("audio: {e}"))
}

/// 16-bit PCM WAV bytes.
pub fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Standard base64 (for handing WAV bytes to the web view).
pub fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= c.len() { T[(n >> shift & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// The lowest sample rate [`read_wav`] accepts (lower ones would make [`resample`] blow up).
pub const MIN_RATE: u32 = 1_000;
/// The highest sample rate [`read_wav`] accepts.
pub const MAX_RATE: u32 = 768_000;

/// Little-endian `u16` at `i`, if `b` has the bytes.
fn le16(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]))
}

/// Little-endian `u32` at `i`, if `b` has the bytes.
fn le32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(i)?, *b.get(i + 1)?, *b.get(i + 2)?, *b.get(i + 3)?]))
}

/// Mono samples and their rate from WAV bytes (PCM 8/16/24/32-bit or 32-bit float).
///
/// Hostile input is refused, never trusted: more than
/// [`limits::WAV_BYTES`](crate::untrusted::limits::WAV_BYTES), chunk lengths past the end of
/// the file, a `fmt ` chunk shorter than 16 bytes, or a rate outside
/// [`MIN_RATE`]..=[`MAX_RATE`].
pub fn read_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32)> {
    let bad = || audio_err("not a WAV file this app can read (PCM or 32-bit float)");
    crate::untrusted::check_size("the WAV file", bytes.len(), crate::untrusted::limits::WAV_BYTES)?;
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad());
    }
    let (mut fmt, mut data): (Option<&[u8]>, Option<&[u8]>) = (None, None);
    let mut rest = &bytes[12..];
    while let (Some(id), Some(len)) = (rest.get(..4), le32(rest, 4)) {
        // The length is the file's claim: clamp it to what is really there.
        let after = &rest[8..];
        let len = usize::try_from(len).unwrap_or(usize::MAX);
        let body = &after[..len.min(after.len())];
        match id {
            b"fmt " if body.len() >= 16 => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        rest = after.get(len.saturating_add(len & 1)..).unwrap_or(&[]);
    }
    let (f, data) = (fmt.ok_or_else(bad)?, data.ok_or_else(bad)?);
    let field16 = |i| le16(f, i).ok_or_else(bad);
    let mut format = field16(0)?;
    let channels = field16(2)?.max(1) as usize;
    let rate = le32(f, 4).ok_or_else(bad)?;
    let bits = field16(14)? as usize;
    if format == 0xFFFE {
        // WAVE_FORMAT_EXTENSIBLE: the sub-format's first two bytes.
        format = le16(f, 24).ok_or_else(bad)?;
    }
    let width = bits / 8;
    if width == 0 || !matches!((format, bits), (1, 8 | 16 | 24 | 32) | (3, 32)) {
        return Err(bad());
    }
    if !(MIN_RATE..=MAX_RATE).contains(&rate) {
        return Err(audio_err(format!("a sample rate of {rate} Hz is not supported ({MIN_RATE} to {MAX_RATE})")));
    }
    let sample = |s: &[u8]| -> f32 {
        match (format, bits) {
            (3, _) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            (_, 8) => (s[0] as f32 - 128.0) / 128.0,
            (_, 16) => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
            (_, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            _ => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
        }
    };
    let frame = width * channels;
    let mono = data.chunks_exact(frame).map(|fr| fr.chunks_exact(width).map(sample).sum::<f32>() / channels as f32).collect();
    Ok((mono, rate))
}

/// 16-bit little-endian PCM bytes to samples.
pub fn from_pcm16(bytes: &[u8]) -> Vec<f32> {
    bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b) as f32 / 32768.0).collect()
}

/// Band-limited resampling (a Hann-windowed sinc, 16 zero crossings each side).
/// A zero rate gives no samples.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == 0 || to == 0 {
        return Vec::new();
    }
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    let ratio = to as f64 / from as f64;
    let cutoff = ratio.min(1.0); // low-pass below the lower Nyquist
    const ZEROS: f64 = 16.0;
    let half = (ZEROS / cutoff).ceil() as isize;
    let n_out = (input.len() as f64 * ratio).round() as usize;
    (0..n_out)
        .map(|j| {
            let t = j as f64 / ratio;
            let centre = t.floor() as isize;
            let mut acc = 0.0;
            for k in centre - half + 1..=centre + half {
                let Some(&x) = usize::try_from(k).ok().and_then(|k| input.get(k)) else { continue };
                let d = (t - k as f64) * cutoff;
                if d.abs() >= ZEROS {
                    continue;
                }
                let sinc = if d == 0.0 { 1.0 } else { (std::f64::consts::PI * d).sin() / (std::f64::consts::PI * d) };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * d / ZEROS).cos();
                acc += x as f64 * sinc * window * cutoff;
            }
            acc as f32
        })
        .collect()
}

/// Decode standard base64 (padding optional, whitespace ignored; the URL-safe `-` and `_`
/// are accepted too). Input over
/// [`limits::BASE64_CHARS`](crate::untrusted::limits::BASE64_CHARS) and a lone trailing
/// character (which can hold no byte) are errors.
pub fn base64_decode(s: &str) -> Result<Vec<u8>> {
    crate::untrusted::check_size("the base64 text", s.len(), crate::untrusted::limits::BASE64_CHARS)?;
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let clean: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    for c in clean.chunks(4) {
        if c.len() == 1 {
            return Err(audio_err("bad base64 (cut short)"));
        }
        let mut n = 0u32;
        for (i, &b) in c.iter().enumerate() {
            n |= val(b).ok_or_else(|| audio_err("bad base64"))? << (18 - 6 * i);
        }
        let bytes = n.to_be_bytes();
        out.extend_from_slice(&bytes[1..c.len()]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_wav() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        for s in [&b""[..], b"M", b"Ma", b"Man", b"hello world"] {
            assert_eq!(base64_decode(&base64(s)).unwrap(), s);
        }
        let w = wav(&[0.0, 1.0, -1.0], 24_000);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 6);
        let (back, rate) = read_wav(&w).unwrap();
        assert_eq!(rate, 24_000);
        assert!((back[1] - 1.0).abs() < 1e-3 && (back[2] + 1.0).abs() < 1e-3);
    }

    /// A WAV header with a `fmt ` chunk of `fmt_len` (claimed) and the given body.
    fn header(fmt_len: u32, fmt_body: &[u8]) -> Vec<u8> {
        let mut b = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        b.extend_from_slice(&fmt_len.to_le_bytes());
        b.extend_from_slice(fmt_body);
        b
    }

    #[test]
    fn short_fmt_chunk_is_refused_not_a_panic() {
        // Claims 16 bytes of fmt but the file ends after 4: `u16_at(f + 14)` read past the end.
        let mut b = header(16, &[1, 0, 1, 0]);
        b.extend_from_slice(b"data\0\0\0\0");
        assert!(read_wav(&header(16, &[1, 0, 1, 0])).is_err());
        assert!(read_wav(&b).is_err());
    }

    #[test]
    fn huge_chunk_length_is_clamped() {
        // A chunk length near u32::MAX used to overflow `i + 8 + len` on 32-bit targets.
        let mut b = wav(&[0.5; 4], 16_000);
        b[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
        let (s, _) = read_wav(&b).unwrap();
        assert_eq!(s.len(), 4);
        let mut odd = header(u32::MAX, &[0; 3]);
        odd.extend_from_slice(b"da");
        assert!(read_wav(&odd).is_err());
    }

    #[test]
    fn truncated_chunk_headers_are_ignored() {
        // Fewer than 8 bytes after the last chunk: no read past the end.
        let mut b = wav(&[0.25; 2], 8_000);
        b.extend_from_slice(b"LIS");
        assert_eq!(read_wav(&b).unwrap().0.len(), 2);
        for n in 12..b.len() {
            let _ = read_wav(&b[..n]);
        }
    }

    #[test]
    fn extensible_format_needs_its_subformat() {
        let mut fmt = vec![0xFE, 0xFF, 1, 0];
        fmt.extend_from_slice(&16_000u32.to_le_bytes());
        fmt.extend_from_slice(&[0, 0, 0, 0, 2, 0, 16, 0]);
        let mut b = header(16, &fmt);
        b.extend_from_slice(b"data\x02\0\0\0\0\0");
        assert!(read_wav(&b).is_err(), "a 16-byte extensible fmt has no sub-format");
    }

    #[test]
    fn absurd_sample_rates_are_refused() {
        // Rate 1 Hz made `resample` to 16 kHz allocate 16 000 samples per input sample.
        assert!(read_wav(&wav(&[0.0; 8], 1)).unwrap_err().to_string().contains("sample rate"));
        assert!(read_wav(&wav(&[0.0; 8], MAX_RATE + 1)).is_err());
        assert!(resample(&[0.0; 8], 0, 16_000).is_empty());
        assert!(resample(&[0.0; 8], 16_000, 0).is_empty());
    }

    #[test]
    fn oversized_input_is_refused_before_parsing() {
        let big = "A".repeat(crate::untrusted::limits::BASE64_CHARS + 1);
        assert!(base64_decode(&big).unwrap_err().to_string().contains("too large"));
    }

    #[test]
    fn lone_base64_character_is_an_error() {
        assert!(base64_decode("TWFuT").is_err());
        assert_eq!(base64_decode("TWFu").unwrap(), b"Man");
    }
}
