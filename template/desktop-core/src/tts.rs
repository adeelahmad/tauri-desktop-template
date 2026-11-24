//! Text to speech with tiny-voca (feature `tts`): a 4.5M-parameter English voice model
//! (Apache-2.0, two voices, 24 kHz) run with ONNX Runtime, about 70x real time on a CPU.
//!
//! The text front end is a Rust reading of the dizione phonemizer that the model was
//! trained with: numbers become words, words come from its 125k-word lexicon (IPA with
//! stress marks), plurals and -ed/-ing forms come from their stems, and anything else is
//! spelled out letter by letter. Words are joined with `<sp>`, punctuation is kept, and
//! long text is split into sentences of at most 512 tokens.
//!
//! Files (downloaded once into `<data>/models/tiny-voca`): `model.onnx`, `lexicon.tsv`.
//!
//! Main items: [`Tts`] ([`Tts::load`], [`Tts::speak`]), [`download`], [`TtsConfig`], and the
//! model-free front end [`Phonemizer`] and [`number_words`].
//!
//! # Examples
//!
//! ```
//! use desktop_core::tts::{number_words, TtsConfig};
//!
//! assert_eq!(number_words(42).join(" "), "forty two");
//! let cfg = TtsConfig { speed: 99.0, ..TtsConfig::default() }.validated();
//! assert_eq!(cfg.speed, 4.0); // clamped
//! ```
//!
//! Speaking downloads the voice once:
//!
//! ```no_run
//! use desktop_core::tts::{download, model_dir, Tts, TtsConfig, SAMPLE_RATE};
//!
//! let dir = model_dir("data".as_ref());
//! download(&dir, &mut |file, done, total| println!("{file}: {done}/{total:?}"))?;
//! let tts = Tts::load(&dir, 0)?;
//! let samples = tts.speak("Hello there.", &TtsConfig::default())?;
//! std::fs::write("hello.wav", desktop_core::audio::wav(&samples, SAMPLE_RATE))?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use crate::{Error, Result};
use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The rate of the samples `Tts::speak` returns.
pub const SAMPLE_RATE: u32 = 24_000;
const MAX_TOKENS: usize = 510;
const BASE: &str = "https://huggingface.co/n4ze3m/tiny-voca/resolve/main";
/// The files and their SHA-256 (checked after download).
pub const FILES: [(&str, &str, &str); 2] = [
    ("model.onnx", "onnx/model.onnx", "fe6d918791a15b82e37465981d7f27a070e46b46635e5a3482fc8dff87a1ae46"),
    ("lexicon.tsv", "phonemizer/lexicon.tsv", "3fc930345c6b3cabc533981c4b212fa309eec83913fcdbd42919338270d64b06"),
];

/// The model's symbol table (config.json `vocab`): specials, punctuation, then phones.
const SYMBOLS: [&str; 76] = [
    "<pad>", "<bos>", "<eos>", "<sp>", "<unk>", ",", ".", "?", "!", ";", ":", "\"", "'", "(", ")", "—", "…", "p", "b", "t", "d", "k", "ɡ", "f", "v", "θ", "ð", "s", "z", "ʃ", "ʒ", "h", "m", "n", "ŋ", "l", "ɹ", "j",
    "w", "tʃ", "dʒ", "ɾ", "ʔ", "ə", "ɐ", "ɚ", "ᵻ", "ɪ", "i", "iː", "ɛ", "æ", "ʌ", "ɑː", "ɔ", "ɔː", "ʊ", "uː", "ɜː", "eɪ", "aɪ", "ɔɪ", "oʊ", "aʊ", "iə", "oː", "ɑːɹ", "ɔːɹ", "oːɹ", "ɪɹ", "ɛɹ", "ʊɹ", "aɪɚ", "aɪə",
    "əl", "n̩",
];
const BOS: i64 = 1;
const EOS: i64 = 2;
const SP: i64 = 3;
const UNK: i64 = 4;
const FIRST_PHONE: i64 = 17;

/// Text to speech settings (`tts` in settings).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TtsConfig {
    /// Allow speech (off by default).
    pub enabled: bool,
    /// 0 = F2 (female), 1 = M2 (male).
    pub voice: i64,
    /// 1.0 as trained; larger is faster.
    pub speed: f32,
    /// Added to the log pitch (0 = as trained).
    pub pitch_shift: f32,
    /// Scales pitch movement (1 = as trained).
    pub pitch_range: f32,
    /// Silence between sentences, in milliseconds.
    pub pause_ms: u32,
    /// CPU threads for the model; 0 = ONNX Runtime's choice.
    pub threads: usize,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self { enabled: false, voice: 0, speed: 1.0, pitch_shift: 0.0, pitch_range: 1.0, pause_ms: 250, threads: 0 }
    }
}

impl TtsConfig {
    /// Voice, speed and pitch clamped to what the model handles.
    pub fn validated(mut self) -> Self {
        self.voice = self.voice.clamp(0, 1);
        let fin = |v: f32, d: f32, lo: f32, hi: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.speed = fin(self.speed, 1.0, 0.25, 4.0);
        self.pitch_shift = fin(self.pitch_shift, 0.0, -1.0, 1.0);
        self.pitch_range = fin(self.pitch_range, 1.0, 0.0, 3.0);
        self.pause_ms = self.pause_ms.min(5000);
        self.threads = self.threads.min(64);
        self
    }
}

fn tts_err(e: impl std::fmt::Display) -> Error {
    Error::Source(format!("speech: {e}"))
}

/// One word: (symbol id, stress 0/1/2) per phone.
type Phones = Vec<(i64, i64)>;

/// The text front end (no model needed, so it is tested on its own).
pub struct Phonemizer {
    lexicon: HashMap<String, Phones>,
}

fn phone_id(p: &str) -> Option<i64> {
    SYMBOLS.iter().skip(FIRST_PHONE as usize).position(|s| *s == p).map(|i| i as i64 + FIRST_PHONE)
}

/// "h ə l ˈoʊ" -> phones with stress.
fn parse_entry(ipa: &str) -> Phones {
    ipa.split_whitespace()
        .map(|t| {
            let (stress, p) = if let Some(p) = t.strip_prefix('ˈ') {
                (1, p)
            } else if let Some(p) = t.strip_prefix('ˌ') {
                (2, p)
            } else {
                (0, t)
            };
            (phone_id(p).unwrap_or(UNK), stress)
        })
        .collect()
}

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

/// 1234 -> "one thousand two hundred thirty four".
pub fn number_words(mut n: u64) -> Vec<String> {
    fn below_thousand(n: u64, out: &mut Vec<String>) {
        if n >= 100 {
            out.push(ONES[(n / 100) as usize].into());
            out.push("hundred".into());
        }
        let r = n % 100;
        if r >= 20 {
            out.push(TENS[(r / 10) as usize].into());
            if !r.is_multiple_of(10) {
                out.push(ONES[(r % 10) as usize].into());
            }
        } else if r > 0 || n == 0 {
            out.push(ONES[r as usize].into());
        }
    }
    if n == 0 {
        return vec!["zero".into()];
    }
    let mut out = Vec::new();
    for (scale, name) in [(1_000_000_000_000u64, "trillion"), (1_000_000_000, "billion"), (1_000_000, "million"), (1_000, "thousand")] {
        if n >= scale {
            below_thousand(n / scale, &mut out);
            out.push(name.into());
            n %= scale;
        }
    }
    if n > 0 {
        below_thousand(n, &mut out);
    }
    out
}

/// A piece of text: a word to say or punctuation to keep.
#[derive(Debug, PartialEq)]
enum Piece {
    Word(String),
    Punct(i64),
}

fn punct_id(c: char) -> Option<i64> {
    let s = match c {
        '“' | '”' => "\"",
        '‘' | '’' => "'",
        '–' => "—",
        _ => return SYMBOLS[5..17].iter().position(|s| s.chars().eq(std::iter::once(c))).map(|i| i as i64 + 5),
    };
    SYMBOLS.iter().position(|x| *x == s).map(|i| i as i64)
}

/// Split text into words (numbers spelled out) and punctuation.
fn pieces(text: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || (chars[i] == ',' && chars.get(i + 1).is_some_and(char::is_ascii_digit))) {
                i += 1;
            }
            let whole: String = chars[start..i].iter().filter(|c| c.is_ascii_digit()).collect();
            out.extend(whole.parse::<u64>().map(number_words).unwrap_or_else(|_| whole.chars().map(|d| ONES[d as usize - '0' as usize].to_string()).collect()).into_iter().map(Piece::Word));
            // Decimals: "3.11" -> three point one one.
            if chars.get(i) == Some(&'.') && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
                out.push(Piece::Word("point".into()));
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    out.push(Piece::Word(ONES[chars[i] as usize - '0' as usize].into()));
                    i += 1;
                }
            }
            if chars.get(i) == Some(&'%') {
                out.push(Piece::Word("percent".into()));
                i += 1;
            }
        } else if c.is_alphabetic() || ((c == '\'' || c == '’') && i > 0 && chars[i - 1].is_alphabetic() && chars.get(i + 1).is_some_and(|n| n.is_alphabetic())) {
            let start = i;
            while i < chars.len() && (chars[i].is_alphabetic() || ((chars[i] == '\'' || chars[i] == '’') && chars.get(i + 1).is_some_and(|n| n.is_alphabetic()))) {
                i += 1;
            }
            out.push(Piece::Word(chars[start..i].iter().collect::<String>().replace('’', "'").to_lowercase()));
        } else {
            if let Some(p) = punct_id(c) {
                out.push(Piece::Punct(p));
            } else if c == '&' {
                out.push(Piece::Word("and".into()));
            } else if c == '-' && chars.get(i + 1) == Some(&'-') {
                out.push(Piece::Punct(15));
            }
            i += 1;
        }
    }
    out
}

impl Phonemizer {
    /// Read the lexicon file (`lexicon.tsv`).
    pub fn load(lexicon_tsv: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(lexicon_tsv).map_err(|e| tts_err(format!("{}: {e}", lexicon_tsv.display())))?;
        Ok(Self::from_tsv(&text))
    }

    /// Build from lexicon text: one `word<TAB>ipa` per line.
    pub fn from_tsv(text: &str) -> Self {
        let lexicon = text.lines().filter_map(|l| l.split_once('\t')).map(|(w, ipa)| (w.to_string(), parse_entry(ipa))).collect();
        Self { lexicon }
    }

    /// Letter names, joined; the last letter carries the main stress.
    fn spell(&self, word: &str) -> Phones {
        let names: Vec<Phones> = word
            .chars()
            .filter(char::is_ascii_alphabetic)
            .map(|c| if c == 'a' { parse_entry("ˈeɪ") } else { self.lexicon.get(&c.to_string()).cloned().unwrap_or_default() })
            .collect();
        let n = names.len();
        names.into_iter().enumerate().flat_map(|(i, name)| name.into_iter().map(move |(p, s)| (p, if s == 1 && i + 1 < n { 2 } else { s }))).collect()
    }

    fn word(&self, w: &str) -> Phones {
        if let Some(p) = self.lexicon.get(w) {
            return p.clone();
        }
        let id = |p: &str| phone_id(p).unwrap_or(UNK);
        // A stem the lexicon knows plus a regular ending.
        let endings: [(&str, &str, &[&str]); 5] = [("'s", "", &["z"]), ("s", "", &["z"]), ("es", "", &["ᵻ", "z"]), ("ed", "", &["d"]), ("ing", "", &["ɪ", "ŋ"])];
        for (end, add, tail) in endings {
            if let Some(stem) = w.strip_suffix(end).filter(|s| s.len() > 2) {
                for candidate in [format!("{stem}{add}"), format!("{stem}e")] {
                    if let Some(p) = self.lexicon.get(&candidate) {
                        let mut out = p.clone();
                        out.extend(tail.iter().map(|t| (id(t), 0)));
                        return out;
                    }
                }
            }
        }
        self.spell(w)
    }

    /// Token ids and stress for each sentence (each at most 512 tokens, each ending in
    /// punctuation so the last word is not cut off).
    pub fn sentences(&self, text: &str) -> Vec<(Vec<i64>, Vec<i64>)> {
        let mut out = Vec::new();
        let (mut ids, mut stress) = (vec![BOS], vec![0]);
        let is_phone = |id: i64| id >= FIRST_PHONE || id == UNK;
        let mut flush = |ids: &mut Vec<i64>, stress: &mut Vec<i64>| {
            if ids.iter().any(|i| is_phone(*i)) {
                if !(5..=16).contains(ids.last().unwrap_or(&0)) {
                    ids.push(6);
                    stress.push(0);
                }
                ids.push(EOS);
                stress.push(0);
                out.push((std::mem::replace(ids, vec![BOS]), std::mem::replace(stress, vec![0])));
            } else {
                *ids = vec![BOS];
                *stress = vec![0];
            }
        };
        for piece in pieces(text) {
            match piece {
                Piece::Punct(p) => {
                    if ids.last() != Some(&p) && ids.len() > 1 {
                        ids.push(p);
                        stress.push(0);
                    }
                    if [6, 7, 8].contains(&p) {
                        flush(&mut ids, &mut stress);
                    }
                }
                Piece::Word(w) => {
                    let phones = self.word(&w);
                    if phones.is_empty() {
                        continue;
                    }
                    if ids.len() + phones.len() + 3 > MAX_TOKENS {
                        flush(&mut ids, &mut stress);
                    }
                    if ids.last().is_some_and(|i| is_phone(*i)) {
                        ids.push(SP);
                        stress.push(0);
                    }
                    for (p, s) in phones {
                        ids.push(p);
                        stress.push(s);
                    }
                }
            }
        }
        flush(&mut ids, &mut stress);
        out
    }
}

/// The voice model plus its front end, loaded once.
pub struct Tts {
    phonemizer: Phonemizer,
    session: Mutex<Session>,
}

impl Tts {
    /// Load from a folder holding the `FILES`.
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        let phonemizer = Phonemizer::load(&dir.join("lexicon.tsv"))?;
        let mut builder = Session::builder(&crate::onnx_env()?).map_err(tts_err)?;
        if threads > 0 {
            builder = builder.with_intra_threads(threads).map_err(tts_err)?;
        }
        let session = builder.commit_from_file(dir.join("model.onnx")).map_err(tts_err)?;
        Ok(Self { phonemizer, session: Mutex::new(session) })
    }

    /// Mono f32 samples at `SAMPLE_RATE`.
    pub fn speak(&self, text: &str, cfg: &TtsConfig) -> Result<Vec<f32>> {
        let mut audio = Vec::new();
        let pause = vec![0.0f32; (SAMPLE_RATE as usize * cfg.pause_ms as usize) / 1000];
        let mut session = self.session.lock().map_err(tts_err)?;
        for (ids, stress) in self.phonemizer.sentences(text) {
            let n = ids.len();
            let one = |v: f32| Tensor::from_array(([1usize], vec![v])).map_err(tts_err);
            let outputs = session
                .run(ort::inputs![
                    "tokens" => Tensor::from_array(([1usize, n], ids)).map_err(tts_err)?,
                    "stress" => Tensor::from_array(([1usize, n], stress)).map_err(tts_err)?,
                    "voice" => Tensor::from_array(([1usize], vec![cfg.voice])).map_err(tts_err)?,
                    "speed" => one(cfg.speed)?,
                    "pitch_shift" => one(cfg.pitch_shift)?,
                    "pitch_range" => one(cfg.pitch_range)?,
                ])
                .map_err(tts_err)?;
            let (_, samples) = outputs["audio"].try_extract_tensor::<f32>().map_err(tts_err)?;
            if !audio.is_empty() {
                audio.extend_from_slice(&pause);
            }
            audio.extend_from_slice(samples);
        }
        // The model can peak a little above 1.0.
        let peak = audio.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        if peak > 0.99 {
            let g = 0.99 / peak;
            audio.iter_mut().for_each(|s| *s *= g);
        }
        Ok(audio)
    }
}

/// The model folder inside the app's data folder.
pub fn model_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("models").join("tiny-voca")
}

/// Download the model files that are missing (resumable), checking each SHA-256.
pub fn download(dir: &Path, progress: &mut dyn FnMut(&str, u64, Option<u64>)) -> Result<()> {
    use sha2::Digest;
    for (name, remote, sha) in FILES {
        let dest = dir.join(name);
        if dest.is_file() {
            continue;
        }
        let tmp = dir.join(format!("{name}.download"));
        crate::net::download_to(&format!("{BASE}/{remote}"), &tmp, &mut |d, t| progress(name, d, t)).map_err(tts_err)?;
        let got = format!("{:x}", sha2::Sha256::digest(std::fs::read(&tmp)?));
        if got != sha {
            let _ = std::fs::remove_file(&tmp);
            return Err(tts_err(format!("{name} failed its checksum")));
        }
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEX: &str = "hello\th ə l ˈoʊ\nsamir\ts ə m ˈɪɹ\nit\tɪ t\ncost\tk ˈɔ s t\nthree\tθ ɹ ˈiː\ndollars\td ˈɑː l ɚ z\nbook\tb ˈʊ k\nb\tb ˈiː\nc\ts ˈiː\nwalk\tw ˈɔː k\n";

    fn marked(p: &Phonemizer, text: &str) -> Vec<String> {
        p.sentences(text)
            .into_iter()
            .map(|(ids, st)| ids.iter().zip(st).map(|(i, s)| format!("{}{}", ["", "ˈ", "ˌ"][s as usize], SYMBOLS[*i as usize])).collect::<Vec<_>>().join(" "))
            .collect()
    }

    #[test]
    fn sequences_match_dizione() {
        let p = Phonemizer::from_tsv(LEX);
        // dizione's own test: "Hello Samir" -> h ə l ˈoʊ <sp> s ə m ˈɪɹ
        assert_eq!(marked(&p, "Hello Samir"), ["<bos> h ə l ˈoʊ <sp> s ə m ˈɪɹ . <eos>"]);
        assert_eq!(marked(&p, "Hello, it cost 3 dollars!! Book."), ["<bos> h ə l ˈoʊ , ɪ t <sp> k ˈɔ s t <sp> θ ɹ ˈiː <sp> d ˈɑː l ɚ z ! <eos>", "<bos> b ˈʊ k . <eos>"]);
        // Unknown words: a known stem plus an ending, else the letters.
        assert_eq!(marked(&p, "walked"), ["<bos> w ˈɔː k d . <eos>"]);
        assert_eq!(marked(&p, "BC"), ["<bos> b ˌiː s ˈiː . <eos>"]);
        assert!(marked(&p, "?!").is_empty());
    }

    #[test]
    fn numbers_and_wav() {
        assert_eq!(number_words(1234).join(" "), "one thousand two hundred thirty four");
        assert_eq!(number_words(2_000_005).join(" "), "two million five");
        assert_eq!(number_words(0), ["zero"]);
    }

    /// Runs when `APP_TEST_TTS_DIR` holds model.onnx and lexicon.tsv.
    #[test]
    fn speaks_with_the_real_model() {
        let Some(dir) = std::env::var_os("APP_TEST_TTS_DIR").map(PathBuf::from) else {
            eprintln!("skipped: set APP_TEST_TTS_DIR");
            return;
        };
        let tts = Tts::load(&dir, 0).unwrap();
        let audio = tts.speak("Hello there. The heart pumps blood through the body.", &TtsConfig::default()).unwrap();
        let secs = audio.len() as f32 / SAMPLE_RATE as f32;
        assert!((1.0..15.0).contains(&secs), "{secs} s");
        assert!(audio.iter().any(|s| s.abs() > 0.05), "silent");
        if let Some(out) = std::env::var_os("APP_TEST_TTS_WAV") {
            std::fs::write(out, crate::audio::wav(&audio, SAMPLE_RATE)).unwrap();
        }
    }
}
