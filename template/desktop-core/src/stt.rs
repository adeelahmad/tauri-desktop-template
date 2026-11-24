//! Speech to text with OpenAI Whisper (feature `stt`), run with ONNX Runtime: no server,
//! no Python, nothing leaves the machine. Models are the 8-bit ONNX exports from
//! onnx-community (MIT): tiny (40 MB), base (75 MB, the default) and small (250 MB), all
//! multilingual, with language detection and translation to English.
//!
//! Everything around the model is plain Rust: audio (read and resampled to 16 kHz by
//! `crate::audio`) is turned into Whisper's 80-band log-mel spectrogram (a small
//! mixed-radix FFT, Slaney mel filters as librosa makes them) and cut into 30-second
//! windows; text comes from greedy decoding and GPT-2 byte-level token decoding.
//!
//! Files (downloaded once into `<data>/models/whisper-<size>`): `encoder.onnx`,
//! `decoder.onnx`, `vocab.json`, `added_tokens.json`.
//!
//! Main items: [`Stt`] ([`Stt::load`], [`Stt::transcribe`]), [`download`], [`SttConfig`],
//! and the model-free parts [`log_mel`] and [`Tokenizer`].
//!
//! # Examples
//!
//! The front end runs without a model:
//!
//! ```
//! use desktop_core::stt::{log_mel, SttConfig, SAMPLE_RATE};
//!
//! let cfg = SttConfig { model: "huge".into(), ..SttConfig::default() }.validated();
//! assert_eq!(cfg.model, "base"); // unknown sizes fall back to the default
//! let mel = log_mel(&vec![0.0; SAMPLE_RATE as usize]);
//! assert_eq!(mel.len(), 80 * 3000); // 80 mel bands x 3000 frames (30 seconds)
//! ```
//!
//! Transcribing downloads the model once:
//!
//! ```no_run
//! use desktop_core::stt::{download, model_dir, Stt, SttConfig};
//!
//! let cfg = SttConfig { enabled: true, ..SttConfig::default() };
//! let dir = model_dir("data".as_ref(), &cfg.model);
//! download(&dir, &cfg.model, &mut |file, done, total| println!("{file}: {done}/{total:?}"))?;
//! let stt = Stt::load(&dir, &cfg.model, 0)?;
//! let (samples, _rate) = desktop_core::audio::read_wav(&std::fs::read("note.wav")?)?;
//! println!("{}", stt.transcribe(&samples, &cfg)?.text);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::{Error, Result};
use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Whisper's input rate; other audio is resampled to it.
pub const SAMPLE_RATE: u32 = 16_000;
const N_FFT: usize = 400;
const HOP: usize = 160;
const N_MELS: usize = 80;
const FRAMES: usize = 3000;
const WINDOW: usize = SAMPLE_RATE as usize * 30;
/// Tokens per 30-second window (half the decoder's 448, as Whisper does).
const MAX_NEW_TOKENS: usize = 224;

const EOT: i64 = 50257;
const SOT: i64 = 50258;
const TRANSLATE: i64 = 50358;
const TRANSCRIBE: i64 = 50359;
const NO_TIMESTAMPS: i64 = 50363;
/// generation_config.json `suppress_tokens` below the special tokens (the same for every
/// multilingual size): symbols Whisper should not produce.
const SUPPRESS: [usize; 82] = [
    1, 2, 7, 8, 9, 10, 14, 25, 26, 27, 28, 29, 31, 58, 59, 60, 61, 62, 63, 90, 91, 92, 93, 359, 503, 522, 542, 873, 893, 902, 918, 922, 931, 1350, 1853, 1982, 2460, 2627, 3246, 3253, 3268, 3536, 3846, 3961,
    4183, 4667, 6585, 6647, 7273, 9061, 9383, 10428, 10929, 11938, 12033, 12331, 12562, 13793, 14157, 14635, 15265, 15618, 16553, 16604, 18362, 18956, 20075, 21675, 22520, 26130, 26161, 26435, 28279, 29464,
    31650, 32302, 32470, 36865, 42863, 47425, 49870, 50254,
];

/// The model sizes, their pinned Hugging Face revision and the SHA-256 of each file.
pub const MODELS: [(&str, &str, &str, &str); 3] = [
    ("tiny", "ff4177021cc41f7db950912b73ea4fdf7d01d8e7", "2af4a414ca47aa30f61246017e5fe82b0a8d229281d1255ba666a2a7f6b84d19", "2a160a2a5ccf901f83aa4d4ceec580a269bd2cec582871e0816e98c610594f12"),
    ("base", "27a07b4b52b3a4c5886134bb4320e93e1a4bbc7c", "5862993336bf33acd23736071aae2b32261d3b1b2f37780194460d4ef974dd46", "c859636f4463dd5a9ea752546c6c7badeb48ab3e22b340cafef08a15552fcb2e"),
    ("small", "36050c46d777d46dc4b5f43f6d90574fc38f8732", "a43a83f3c5361cd591cfa7c36f14b43cf7cb22f47a415cc14a8d557be800fa92", "0486afa8eb464bf2210d07aca0cb3f5d91d3f2f01cbe47f9f1e6ff9122abaaf6"),
];
const VOCAB_SHA: &str = "50d6a919f0a0601d56a04eb583c780d18553aa388254ba3158eb6a00f13e2c1a";
const ADDED_SHA: &str = "9715fd2243b6f06a5858b5e32950d2853f73dd5bc201aafcf76f5082a2d8acd1";

/// Speech to text settings (`stt` in settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SttConfig {
    /// Allow transcription (off by default).
    pub enabled: bool,
    /// "tiny", "base" or "small".
    pub model: String,
    /// A language code such as "en" or "de"; empty = detect it.
    pub language: String,
    /// Translate into English instead of transcribing.
    pub translate: bool,
    /// Longest recording accepted, in seconds.
    pub max_seconds: u32,
    /// CPU threads for the model; 0 = ONNX Runtime's choice.
    pub threads: usize,
}

impl Default for SttConfig {
    fn default() -> Self {
        Self { enabled: false, model: "base".into(), language: String::new(), translate: false, max_seconds: 600, threads: 0 }
    }
}

impl SttConfig {
    /// An unknown model becomes `base`; limits clamped.
    pub fn validated(mut self) -> Self {
        if !MODELS.iter().any(|m| m.0 == self.model) {
            self.model = "base".into();
        }
        self.language = self.language.trim().to_ascii_lowercase();
        if self.language == "auto" || !self.language.chars().all(|c| c.is_ascii_lowercase()) || self.language.len() > 3 {
            self.language.clear();
        }
        self.max_seconds = self.max_seconds.clamp(5, 7200);
        self.threads = self.threads.min(64);
        self
    }
}

fn stt_err(e: impl std::fmt::Display) -> Error {
    Error::Source(format!("speech to text: {e}"))
}

// ---- Features ----

/// An in-place-free mixed-radix FFT (radix 2 down to an odd length, then a plain DFT):
/// enough for Whisper's 400-point frames (2^4 x 25).
fn fft(re: &[f64], im: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let n = re.len();
    if n % 2 == 1 || n <= 2 {
        let mut out = (vec![0.0; n], vec![0.0; n]);
        for k in 0..n {
            for t in 0..n {
                let a = -2.0 * std::f64::consts::PI * (k * t % n) as f64 / n as f64;
                let (s, c) = a.sin_cos();
                out.0[k] += re[t] * c - im[t] * s;
                out.1[k] += re[t] * s + im[t] * c;
            }
        }
        return out;
    }
    let split = |odd: usize| -> (Vec<f64>, Vec<f64>) { (re.iter().skip(odd).step_by(2).copied().collect(), im.iter().skip(odd).step_by(2).copied().collect()) };
    let ((er, ei), (or, oi)) = (split(0), split(1));
    let ((er, ei), (or, oi)) = (fft(&er, &ei), fft(&or, &oi));
    let mut out = (vec![0.0; n], vec![0.0; n]);
    for k in 0..n / 2 {
        let (s, c) = (-2.0 * std::f64::consts::PI * k as f64 / n as f64).sin_cos();
        let (tr, ti) = (or[k] * c - oi[k] * s, or[k] * s + oi[k] * c);
        out.0[k] = er[k] + tr;
        out.1[k] = ei[k] + ti;
        out.0[k + n / 2] = er[k] - tr;
        out.1[k + n / 2] = ei[k] - ti;
    }
    out
}

fn hz_to_mel(f: f64) -> f64 {
    let (f_sp, min_log_hz, min_log_mel, step) = (200.0 / 3.0, 1000.0, 15.0, (6.4f64).ln() / 27.0);
    if f >= min_log_hz {
        min_log_mel + (f / min_log_hz).ln() / step
    } else {
        f / f_sp
    }
}

fn mel_to_hz(m: f64) -> f64 {
    let (f_sp, min_log_hz, min_log_mel, step) = (200.0 / 3.0, 1000.0, 15.0, (6.4f64).ln() / 27.0);
    if m >= min_log_mel {
        min_log_hz * (step * (m - min_log_mel)).exp()
    } else {
        f_sp * m
    }
}

/// librosa.filters.mel(sr=16000, n_fft=400, n_mels=80) with Slaney scale and norm.
fn mel_filters() -> Vec<[f64; N_FFT / 2 + 1]> {
    let bins = N_FFT / 2 + 1;
    let freqs: Vec<f64> = (0..bins).map(|i| i as f64 * SAMPLE_RATE as f64 / N_FFT as f64).collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(SAMPLE_RATE as f64 / 2.0));
    let mel_f: Vec<f64> = (0..N_MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64)).collect();
    (0..N_MELS)
        .map(|m| {
            let mut row = [0.0; N_FFT / 2 + 1];
            let enorm = 2.0 / (mel_f[m + 2] - mel_f[m]);
            for (b, f) in freqs.iter().enumerate() {
                let lower = (f - mel_f[m]) / (mel_f[m + 1] - mel_f[m]);
                let upper = (mel_f[m + 2] - f) / (mel_f[m + 2] - mel_f[m + 1]);
                row[b] = lower.min(upper).max(0.0) * enorm;
            }
            row
        })
        .collect()
}

/// Whisper's input: an 80 x 3000 log-mel spectrogram of 30 seconds (zero-padded), row-major.
pub fn log_mel(samples: &[f32]) -> Vec<f32> {
    let mut audio = vec![0.0f64; WINDOW];
    audio.iter_mut().zip(samples).for_each(|(a, s)| *a = *s as f64);
    // centre=True: reflect-pad by n_fft / 2 on each side.
    let pad = N_FFT / 2;
    let mut padded = Vec::with_capacity(WINDOW + 2 * pad);
    padded.extend((1..=pad).rev().map(|i| audio[i]));
    padded.extend_from_slice(&audio);
    padded.extend((WINDOW - pad - 1..WINDOW - 1).rev().map(|i| audio[i]));
    let hann: Vec<f64> = (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / N_FFT as f64).cos()).collect();
    let filters = mel_filters();
    let zeros = vec![0.0; N_FFT];
    let mut mel = vec![0.0f64; N_MELS * FRAMES];
    for t in 0..FRAMES {
        let frame: Vec<f64> = padded[t * HOP..t * HOP + N_FFT].iter().zip(&hann).map(|(x, w)| x * w).collect();
        let (re, im) = fft(&frame, &zeros);
        let power: Vec<f64> = (0..=N_FFT / 2).map(|k| re[k] * re[k] + im[k] * im[k]).collect();
        for (m, row) in filters.iter().enumerate() {
            mel[m * FRAMES + t] = row.iter().zip(&power).map(|(w, p)| w * p).sum::<f64>().max(1e-10).log10();
        }
    }
    let top = mel.iter().copied().fold(f64::MIN, f64::max);
    mel.iter().map(|v| ((v.max(top - 8.0) + 4.0) / 4.0) as f32).collect()
}

// ---- Tokens ----

/// GPT-2's byte-to-character table, reversed: the vocabulary's characters back to bytes.
fn unicode_to_byte() -> HashMap<char, u8> {
    let mut keep: Vec<u32> = (b'!' as u32..=b'~' as u32).chain(0xA1..=0xAC).chain(0xAE..=0xFF).collect();
    let mut chars: Vec<u32> = keep.clone();
    let mut n = 0;
    for b in 0..256u32 {
        if !keep.contains(&b) {
            keep.push(b);
            chars.push(256 + n);
            n += 1;
        }
    }
    keep.iter().zip(chars).filter_map(|(b, c)| char::from_u32(c).map(|c| (c, *b as u8))).collect()
}

/// Whisper's byte-level BPE vocabulary, for decoding token ids to text.
pub struct Tokenizer {
    pieces: Vec<Vec<u8>>,
    languages: HashMap<String, i64>,
}

impl Tokenizer {
    /// Build from `vocab.json` and `added_tokens.json`.
    pub fn from_json(vocab: &str, added: &str) -> Result<Self> {
        let vocab: HashMap<String, usize> = serde_json::from_str(vocab)?;
        let added: HashMap<String, i64> = serde_json::from_str(added)?;
        let bytes = unicode_to_byte();
        let mut pieces = vec![Vec::new(); vocab.values().max().map_or(0, |m| m + 1)];
        for (tok, id) in vocab {
            pieces[id] = tok.chars().filter_map(|c| bytes.get(&c).copied()).collect();
        }
        // Language tokens are <|xx|> between <|startoftranscript|> and <|translate|>.
        let languages = added
            .into_iter()
            .filter(|(_, id)| (SOT + 1..TRANSLATE).contains(id))
            .filter_map(|(t, id)| Some((t.strip_prefix("<|")?.strip_suffix("|>")?.to_string(), id)))
            .collect();
        Ok(Self { pieces, languages })
    }

    /// Text for `ids` (special tokens are skipped).
    pub fn decode(&self, ids: &[i64]) -> String {
        let bytes: Vec<u8> = ids.iter().filter(|&&i| i >= 0 && (i as usize) < self.pieces.len() && i < EOT).flat_map(|&i| self.pieces[i as usize].iter().copied()).collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// The token for a language code such as `en`.
    pub fn language(&self, code: &str) -> Option<i64> {
        self.languages.get(code).copied()
    }
}

// ---- Model ----

/// A loaded Whisper model, kept between recordings.
pub struct Stt {
    /// The model size (`tiny`, `base` or `small`).
    pub model: String,
    tokenizer: Tokenizer,
    encoder: Mutex<Session>,
    decoder: Mutex<Session>,
}

/// What `Stt::transcribe` returns.
#[derive(Debug, Clone, Serialize)]
pub struct Transcript {
    /// The text (in English when translating).
    pub text: String,
    /// The language code used (detected or configured).
    pub language: String,
    /// Length of the audio.
    pub seconds: f32,
}

fn argmax(row: &[f32], allowed: impl Fn(usize) -> bool) -> usize {
    row.iter().enumerate().filter(|(i, _)| allowed(*i)).max_by(|a, b| a.1.total_cmp(b.1)).map_or(EOT as usize, |(i, _)| i)
}

impl Stt {
    /// Load from a folder holding the model's files.
    pub fn load(dir: &Path, model: &str, threads: usize) -> Result<Self> {
        let tokenizer = Tokenizer::from_json(&std::fs::read_to_string(dir.join("vocab.json"))?, &std::fs::read_to_string(dir.join("added_tokens.json"))?)?;
        let session = |file: &str| -> Result<Session> {
            let mut b = Session::builder(&crate::onnx_env()?).map_err(stt_err)?;
            if threads > 0 {
                b = b.with_intra_threads(threads).map_err(stt_err)?;
            }
            b.commit_from_file(dir.join(file)).map_err(stt_err)
        };
        Ok(Self { model: model.into(), tokenizer, encoder: Mutex::new(session("encoder.onnx")?), decoder: Mutex::new(session("decoder.onnx")?) })
    }

    /// Logits for the next token after `ids`.
    fn next_logits(&self, hidden: &(Vec<usize>, Vec<f32>), ids: &[i64]) -> Result<Vec<f32>> {
        let mut dec = self.decoder.lock().map_err(stt_err)?;
        let out = dec
            .run(ort::inputs![
                "input_ids" => Tensor::from_array(([1usize, ids.len()], ids.to_vec())).map_err(stt_err)?,
                "encoder_hidden_states" => Tensor::from_array((hidden.0.clone(), hidden.1.clone())).map_err(stt_err)?,
            ])
            .map_err(stt_err)?;
        let (shape, logits) = out["logits"].try_extract_tensor::<f32>().map_err(stt_err)?;
        let vocab = *shape.last().ok_or_else(|| stt_err("no logits"))? as usize;
        Ok(logits[logits.len() - vocab..].to_vec())
    }

    /// Transcribe mono 16 kHz samples, 30 seconds at a time.
    pub fn transcribe(&self, samples: &[f32], cfg: &SttConfig) -> Result<Transcript> {
        let mut language = cfg.language.clone();
        let mut text = String::new();
        for window in samples.chunks(WINDOW) {
            // Silence makes Whisper invent text: skip windows with no signal.
            let rms = (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt();
            if rms < 1e-3 {
                continue;
            }
            let mel = log_mel(window);
            let hidden = {
                let mut enc = self.encoder.lock().map_err(stt_err)?;
                let out = enc.run(ort::inputs!["input_features" => Tensor::from_array(([1usize, N_MELS, FRAMES], mel)).map_err(stt_err)?]).map_err(stt_err)?;
                let (shape, data) = out["last_hidden_state"].try_extract_tensor::<f32>().map_err(stt_err)?;
                (shape.iter().map(|&d| d as usize).collect::<Vec<_>>(), data.to_vec())
            };
            if language.is_empty() || self.tokenizer.language(&language).is_none() {
                let logits = self.next_logits(&hidden, &[SOT])?;
                let langs: Vec<i64> = self.tokenizer.languages.values().copied().collect();
                let best = argmax(&logits, |i| langs.contains(&(i as i64)));
                language = self.tokenizer.languages.iter().find(|(_, &id)| id == best as i64).map(|(c, _)| c.clone()).unwrap_or_else(|| "en".into());
            }
            let lang = self.tokenizer.language(&language).unwrap_or(SOT + 1);
            let mut ids = vec![SOT, lang, if cfg.translate { TRANSLATE } else { TRANSCRIBE }, NO_TIMESTAMPS];
            let prompt = ids.len();
            for step in 0..MAX_NEW_TOKENS {
                let logits = self.next_logits(&hidden, &ids)?;
                let next = argmax(&logits, |i| {
                    let special = i as i64 >= EOT;
                    let first = step == 0 && (i == 220 || i as i64 == EOT); // begin_suppress_tokens
                    !(first || (special && i as i64 != EOT) || SUPPRESS.binary_search(&i).is_ok())
                }) as i64;
                if next == EOT {
                    break;
                }
                ids.push(next);
                // A greedy decoder can loop: stop when the last 8 tokens repeat the 8 before.
                let n = ids.len() - prompt;
                if n >= 16 && ids[ids.len() - 8..] == ids[ids.len() - 16..ids.len() - 8] {
                    ids.truncate(ids.len() - 8);
                    break;
                }
            }
            let part = self.tokenizer.decode(&ids[prompt..]);
            let part = part.trim();
            if !part.is_empty() {
                if !text.is_empty() {
                    text.push(' ');
                }
                text.push_str(part);
            }
        }
        Ok(Transcript { text, language, seconds: samples.len() as f32 / SAMPLE_RATE as f32 })
    }
}

/// The model folder inside the app's data folder.
pub fn model_dir(data_dir: &Path, model: &str) -> PathBuf {
    data_dir.join("models").join(format!("whisper-{model}"))
}

/// Whether every file of `model` is in place.
pub fn is_downloaded(dir: &Path) -> bool {
    ["encoder.onnx", "decoder.onnx", "vocab.json", "added_tokens.json"].iter().all(|f| dir.join(f).is_file())
}

/// Download the model's missing files (resumable), checking each SHA-256.
pub fn download(dir: &Path, model: &str, progress: &mut dyn FnMut(&str, u64, Option<u64>)) -> Result<()> {
    use sha2::Digest;
    let (_, rev, enc, dec) = MODELS.iter().find(|m| m.0 == model).ok_or_else(|| stt_err(format!("unknown model {model}")))?;
    let files = [("encoder.onnx", "onnx/encoder_model_quantized.onnx", *enc), ("decoder.onnx", "onnx/decoder_model_quantized.onnx", *dec), ("vocab.json", "vocab.json", VOCAB_SHA), ("added_tokens.json", "added_tokens.json", ADDED_SHA)];
    std::fs::create_dir_all(dir)?;
    for (name, remote, sha) in files {
        let dest = dir.join(name);
        if dest.is_file() {
            continue;
        }
        let tmp = dir.join(format!("{name}.download"));
        let url = format!("https://huggingface.co/onnx-community/whisper-{model}/resolve/{rev}/{remote}");
        crate::net::download_to(&url, &tmp, &mut |d, t| progress(name, d, t)).map_err(stt_err)?;
        let got = format!("{:x}", sha2::Sha256::digest(std::fs::read(&tmp)?));
        if got != sha {
            let _ = std::fs::remove_file(&tmp);
            return Err(stt_err(format!("{name} failed its checksum")));
        }
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_matches_a_plain_dft() {
        let re: Vec<f64> = (0..N_FFT).map(|i| ((i * 7 % 13) as f64 - 6.0) / 6.0).collect();
        let (fr, fi) = fft(&re, &vec![0.0; N_FFT]);
        for k in [0, 1, 37, 200, 399] {
            let (mut r, mut i) = (0.0, 0.0);
            for (t, x) in re.iter().enumerate() {
                let a = -2.0 * std::f64::consts::PI * (k * t) as f64 / N_FFT as f64;
                r += x * a.cos();
                i += x * a.sin();
            }
            assert!((fr[k] - r).abs() < 1e-6 && (fi[k] - i).abs() < 1e-6, "bin {k}");
        }
    }

    #[test]
    fn mel_filters_match_librosa() {
        let f = mel_filters();
        // librosa.filters.mel(sr=16000, n_fft=400, n_mels=80): row 0 peaks at bin 1, every row is used.
        assert!((f[0][1] - 0.024_862_594).abs() < 1e-6, "{}", f[0][1]);
        assert!(f.iter().all(|r| r.iter().any(|&w| w > 0.0)));
        let mel = log_mel(&[0.0; 100]);
        assert_eq!(mel.len(), N_MELS * FRAMES);
        assert!(mel.iter().all(|&v| (v - -1.5).abs() < 1e-6)); // silence: (-10 + 4) / 4
    }

    #[test]
    fn audio_and_tokens() {
        let tone: Vec<f32> = (0..24_000).map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 24_000.0).sin() * 0.5).collect();
        let (back, rate) = crate::audio::read_wav(&crate::audio::wav(&tone, 24_000)).unwrap();
        assert_eq!((back.len(), rate), (24_000, 24_000));
        let r = crate::audio::resample(&back, 24_000, SAMPLE_RATE);
        assert_eq!(r.len(), 16_000);
        let peak = r[2000..14000].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((peak - 0.5).abs() < 0.02, "{peak}");
        assert_eq!(crate::audio::from_pcm16(&[0, 0x40, 0, 0xC0]), [0.5, -0.5]);
        let t = Tokenizer::from_json(r#"{"Hello":0,"Ġworld":1,"Ã©":2}"#, r#"{"<|en|>":50259,"<|de|>":50261,"<|translate|>":50358}"#).unwrap();
        assert_eq!(t.decode(&[0, 1, 2, EOT]), "Hello worldé");
        assert_eq!((t.language("de"), t.language("translate")), (Some(50261), None));
    }

    /// English speech for the test: `APP_TEST_STT_WAV`, else (with feature `tts`) the
    /// built-in voice reading a sentence when `APP_TEST_TTS_DIR` is set.
    fn test_speech() -> Option<(Vec<f32>, u32)> {
        if let Some(wav) = std::env::var_os("APP_TEST_STT_WAV") {
            return Some(crate::audio::read_wav(&std::fs::read(wav).unwrap()).unwrap());
        }
        #[cfg(feature = "tts")]
        if let Some(dir) = std::env::var_os("APP_TEST_TTS_DIR") {
            let tts = crate::tts::Tts::load(Path::new(&dir), 0).unwrap();
            let audio = tts.speak("Hello there. The heart pumps blood through the body.", &crate::tts::TtsConfig::default()).unwrap();
            return Some((audio, crate::tts::SAMPLE_RATE));
        }
        None
    }

    /// Runs when `APP_TEST_STT_DIR` holds the base model and there is test speech (above);
    /// `APP_TEST_STT_EXPECT` is a word the text must contain.
    #[test]
    fn transcribes_with_the_real_model() {
        let (Some(dir), Some((audio, rate))) = (std::env::var_os("APP_TEST_STT_DIR"), test_speech()) else {
            eprintln!("skipped: set APP_TEST_STT_DIR and APP_TEST_STT_WAV (or APP_TEST_TTS_DIR)");
            return;
        };
        let stt = Stt::load(Path::new(&dir), "base", 0).unwrap();
        let t = stt.transcribe(&crate::audio::resample(&audio, rate, SAMPLE_RATE), &SttConfig::default()).unwrap();
        eprintln!("{t:?}");
        assert_eq!(t.language, "en");
        let expect = std::env::var("APP_TEST_STT_EXPECT").unwrap_or_else(|_| "heart".into());
        assert!(t.text.to_lowercase().contains(&expect), "{}", t.text);
    }
}
