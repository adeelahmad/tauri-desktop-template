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


/// Mono samples and their rate from WAV bytes (PCM 8/16/24/32-bit or 32-bit float).
pub fn read_wav(bytes: &[u8]) -> Result<(Vec<f32>, u32)> {
    let bad = || audio_err("not a WAV file this app can read (PCM or 32-bit float)");
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(bad());
    }
    let u16_at = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
    let (mut fmt, mut data) = (None, None);
    let mut i = 12;
    while i + 8 <= bytes.len() {
        let len = u32_at(i + 4) as usize;
        let body = i + 8..(i + 8 + len).min(bytes.len());
        match &bytes[i..i + 4] {
            b"fmt " if len >= 16 => fmt = Some(body.start),
            b"data" => data = Some(body.clone()),
            _ => {}
        }
        i = body.start + len + (len & 1);
    }
    let (f, data) = (fmt.ok_or_else(bad)?, data.ok_or_else(bad)?);
    let mut format = u16_at(f);
    let channels = u16_at(f + 2).max(1) as usize;
    let rate = u32_at(f + 4);
    let bits = u16_at(f + 14) as usize;
    if format == 0xFFFE && f + 26 <= bytes.len() {
        format = u16_at(f + 24); // WAVE_FORMAT_EXTENSIBLE: the sub-format's first two bytes
    }
    let width = bits / 8;
    if rate == 0 || width == 0 || !matches!((format, bits), (1, 8 | 16 | 24 | 32) | (3, 32)) {
        return Err(bad());
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
    let mono = bytes[data].chunks_exact(frame).map(|fr| fr.chunks_exact(width).map(sample).sum::<f32>() / channels as f32).collect();
    Ok((mono, rate))
}

/// 16-bit little-endian PCM bytes to samples.
pub fn from_pcm16(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0).collect()
}

/// Band-limited resampling (a Hann-windowed sinc, 16 zero crossings each side).
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
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

/// Decode standard base64 (padding optional, whitespace ignored).
pub fn base64_decode(s: &str) -> Result<Vec<u8>> {
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
}
