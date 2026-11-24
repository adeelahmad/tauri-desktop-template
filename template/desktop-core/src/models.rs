//! Models built into the installer ("bundled models"), and where each model is loaded from.
//!
//! By default every model (the embedding model, Whisper, the voice, a GGUF for llama.cpp)
//! downloads on first use into `<data>/models`. An app generated with `models = bundle` or
//! `bundle_with_fallback` ships some of them inside the installer instead: they are listed in
//! `src-tauri/resources/models/models.lock`, fetched into that folder before a build
//! (`kitchen models fetch` or `make models`), and Tauri copies the folder into the app's
//! resources (`bundle.resources` in `tauri.conf.json`).
//!
//! At run time [`locate`] picks the folder a model loads from, in this order:
//!
//! 1. the bundled copy, when the lock lists the model and every file is there with the right
//!    size; its SHA-256 is checked once and a marker in `<data>/models/.bundled` records it;
//! 2. the user's cache (`<data>/models/...`), downloading into it when the network policy
//!    ([`crate::net`]) allows.
//!
//! With [`Mode::Bundle`], a model the lock lists but the installer lacks is an error (the
//! install is broken); with [`Mode::BundleWithFallback`] it downloads instead. The mode is
//! chosen when the app is generated (`APP_MODELS` in `.cargo/config.toml`, overridable at
//! run time with the same environment variable).
//!
//! The lock is plain text, one file per line: `kind model file bytes sha256 url`. Kinds are
//! `embed`, `stt`, `tts` and `llm`. Embedding models are kept in the Hugging Face cache
//! layout fastembed reads (`models--<org>--<repo>/snapshots/<rev>/...`); [`fetch`] writes
//! the `refs/main` file that layout needs.
//!
//! # Example
//!
//! A bundle with one (tiny, fake) model, found with no network at all:
//!
//! ```
//! use desktop_core::models::{self, Lock, Mode, Resolver, Source};
//! use sha2::Digest;
//!
//! let root = std::env::temp_dir().join(format!("models-doc-{}", std::process::id()));
//! let bundle = root.join("resources/models");
//! std::fs::create_dir_all(bundle.join("tiny-voca"))?;
//! std::fs::write(bundle.join("tiny-voca/model.onnx"), b"fake")?;
//! let sha = format!("{:x}", sha2::Sha256::digest(b"fake"));
//! let lock = Lock::parse(&format!("tts tiny-voca model.onnx 4 {sha} https://example.com/model.onnx\n"))?;
//! lock.save(&bundle.join(models::LOCK_FILE))?;
//!
//! let r = Resolver { mode: Mode::Bundle, bundle: Some(bundle.clone()), state_dir: root.join("data/models") };
//! let src = r.locate("tiny-voca", root.join("data/models/tiny-voca"))?;
//! assert_eq!(src, Source::Bundled(bundle.join("tiny-voca")));
//! // A model the lock does not list comes from the user's cache, as before.
//! assert!(matches!(r.locate("whisper-base", root.join("w"))?, Source::User(_)));
//! # std::fs::remove_dir_all(&root)?;
//! # Ok::<(), desktop_core::Error>(())
//! ```

use crate::{Error, Result};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

/// The lock's file name, inside the bundled models folder.
pub const LOCK_FILE: &str = "models.lock";
/// Installers above this size get a warning when models are fetched (about 200 MB).
pub const WARN_BYTES: u64 = 200 * 1000 * 1000;
/// GitHub refuses release assets above 2 GiB, so an installer must stay below it.
pub const RELEASE_ASSET_LIMIT: u64 = 2 * 1024 * 1024 * 1024;

/// How the app gets its models (the `models` answer when the app was generated).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Every model downloads on first use (the default).
    #[default]
    Download,
    /// The bundled models must be in the installer; a missing one is an error.
    Bundle,
    /// Use the bundled models, and download any that are missing.
    BundleWithFallback,
}

impl Mode {
    /// Parse `download`, `bundle` or `bundle_with_fallback`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "download" | "" => Some(Self::Download),
            "bundle" => Some(Self::Bundle),
            "bundle_with_fallback" => Some(Self::BundleWithFallback),
            _ => None,
        }
    }

    /// The mode in force: `APP_MODELS` at run time, else the one chosen when the app was
    /// generated, else [`Mode::Download`].
    pub fn configured() -> Self {
        std::env::var("APP_MODELS")
            .ok()
            .and_then(|m| Self::parse(&m))
            .or_else(|| option_env!("APP_MODELS").and_then(Self::parse))
            .unwrap_or_default()
    }
}

/// What a model is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// A fastembed embedding model (feature `embed`).
    Embed,
    /// A Whisper model (feature `stt`).
    Stt,
    /// The text-to-speech voice (feature `tts`).
    Tts,
    /// A GGUF model for llama.cpp (feature `llama`).
    Llm,
}

impl Kind {
    /// The word used in the lock.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Embed => "embed",
            Self::Stt => "stt",
            Self::Tts => "tts",
            Self::Llm => "llm",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [Self::Embed, Self::Stt, Self::Tts, Self::Llm].into_iter().find(|k| k.as_str() == s)
    }
}

/// One file of a bundled model, as the lock lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedFile {
    /// What the model is for.
    pub kind: Kind,
    /// The model's name, also its folder (`whisper-base`, `embed-bge-small-en-v1.5`).
    pub model: String,
    /// The file's path inside the model folder (forward slashes).
    pub path: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256, lower-case hex.
    pub sha256: String,
    /// Where [`fetch`] downloads it from.
    pub url: String,
}

/// The parsed `models.lock`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lock {
    /// Every file, in the lock's order.
    pub files: Vec<LockedFile>,
}

fn safe_name(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// A relative path with no `..`, no root and no backslashes.
fn safe_path(s: &str) -> bool {
    !s.contains('\\') && !s.is_empty() && Path::new(s).components().all(|c| matches!(c, Component::Normal(_))) && s.split('/').all(|p| !p.is_empty())
}

impl Lock {
    /// Parse the lock's text; `#` starts a comment line.
    pub fn parse(text: &str) -> Result<Self> {
        let mut files = Vec::new();
        for (n, line) in text.lines().enumerate().map(|(n, l)| (n + 1, l.trim())) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let bad = |why: &str| Error::Source(format!("{LOCK_FILE} line {n}: {why}"));
            let f: Vec<&str> = line.split_whitespace().collect();
            let [kind, model, path, size, sha, url] = f[..] else { return Err(bad("expected `kind model file bytes sha256 url`")) };
            let kind = Kind::parse(kind).ok_or_else(|| bad("kind is one of embed, stt, tts, llm"))?;
            if !safe_name(model) || !safe_path(path) {
                return Err(bad("the model name and file path must be plain relative names"));
            }
            let size = size.parse().map_err(|_| bad("the size is a number of bytes"))?;
            if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(bad("the checksum is 64 hex digits"));
            }
            files.push(LockedFile { kind, model: model.into(), path: path.into(), size, sha256: sha.to_ascii_lowercase(), url: url.into() });
        }
        Ok(Self { files })
    }

    /// Read and parse `path`.
    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    /// Write the lock to `path` (with a header comment).
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut s = String::from("# Models built into the installer (docs/GUIDE.md, \"Bundling models\").\n# kind model file bytes sha256 url\n");
        for f in &self.files {
            s.push_str(&format!("{} {} {} {} {} {}\n", f.kind.as_str(), f.model, f.path, f.size, f.sha256, f.url));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        Ok(std::fs::write(path, s)?)
    }

    /// The model names, each once, in order.
    pub fn models(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for f in &self.files {
            if !out.contains(&f.model.as_str()) {
                out.push(&f.model);
            }
        }
        out
    }

    /// The files of `model`.
    pub fn files_of<'a>(&'a self, model: &'a str) -> impl Iterator<Item = &'a LockedFile> + 'a {
        self.files.iter().filter(move |f| f.model == model)
    }

    /// The total size of every file, in bytes (what the models add to the installer).
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

/// The SHA-256 of a file, read in pieces.
pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::Digest;
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(format!("{:x}", h.finalize()))
}

/// Something wrong with a model folder ([`verify`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The model.
    pub model: String,
    /// The file, inside the model folder.
    pub path: String,
    /// What is wrong, in a few words.
    pub what: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}: {}", self.model, self.path, self.what)
    }
}

fn check_file(dir: &Path, f: &LockedFile, checksum: bool) -> Option<String> {
    let p = dir.join(&f.model).join(&f.path);
    match std::fs::metadata(&p) {
        Err(_) => Some("missing".into()),
        Ok(m) if m.len() != f.size => Some(format!("{} bytes, the lock says {}", m.len(), f.size)),
        Ok(_) if checksum => match sha256_file(&p) {
            Ok(sha) if sha == f.sha256 => None,
            Ok(_) => Some("wrong checksum".into()),
            Err(e) => Some(e.to_string()),
        },
        Ok(_) => None,
    }
}

/// Check every file of `lock` under `dir` (the bundled models folder): missing files and
/// wrong sizes, and with `checksum` also each SHA-256. Empty when all is well.
pub fn verify(lock: &Lock, dir: &Path, checksum: bool) -> Vec<Problem> {
    lock.files
        .iter()
        .filter_map(|f| check_file(dir, f, checksum).map(|what| Problem { model: f.model.clone(), path: f.path.clone(), what }))
        .collect()
}

/// Download every file of `lock` that is missing or wrong under `dir` (resuming partial
/// downloads), checking each SHA-256, and write the Hugging Face `refs/main` files.
/// `progress(file, done, total)` runs as data arrives. Returns one line per file.
pub fn fetch(lock: &Lock, dir: &Path, progress: &mut dyn FnMut(&str, u64, Option<u64>)) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for f in &lock.files {
        let dest = dir.join(&f.model).join(&f.path);
        let label = format!("{}/{}", f.model, f.path);
        if check_file(dir, f, true).is_none() {
            out.push(format!("ok       {label}"));
            continue;
        }
        let _ = std::fs::remove_file(&dest);
        crate::net::download_to(&f.url, &dest, &mut |d, t| progress(&label, d, t)).map_err(|e| Error::Source(format!("{label}: {e}")))?;
        if let Some(what) = check_file(dir, f, true) {
            let _ = std::fs::remove_file(&dest);
            return Err(Error::Source(format!("{label}: {what} after download (the lock or the server changed)")));
        }
        out.push(format!("fetched  {label} ({} MB)", f.size / 1_000_000));
    }
    write_hf_refs(lock, dir)?;
    Ok(out)
}

/// For files kept as `<repo>/snapshots/<rev>/...` (the Hugging Face cache layout), write
/// `<repo>/refs/main` = `<rev>`, so the cache finds them without asking the network.
fn write_hf_refs(lock: &Lock, dir: &Path) -> Result<()> {
    for f in &lock.files {
        let parts: Vec<&str> = f.path.split('/').collect();
        if let [repo, "snapshots", rev, _, ..] = parts[..] {
            let refs = dir.join(&f.model).join(repo).join("refs");
            std::fs::create_dir_all(&refs)?;
            std::fs::write(refs.join("main"), rev)?;
        }
    }
    Ok(())
}

static BUNDLE_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Tell this process where the app's bundled models are (the app calls it at start with
/// Tauri's resource folder). The first call wins.
pub fn set_bundle_dir(dir: PathBuf) {
    let _ = BUNDLE_DIR.set(dir);
}

/// The bundled models folder (one holding a `models.lock`): the one set with
/// [`set_bundle_dir`], `APP_MODELS_DIR`, or the usual places next to the executable
/// (`models/`, macOS `../Resources/models/`, Linux packages `../lib/<app>/models/`).
pub fn bundle_dir() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = BUNDLE_DIR.get().cloned().into_iter().collect();
    if let Some(d) = std::env::var_os("APP_MODELS_DIR").filter(|d| !d.is_empty()) {
        candidates.push(d.into());
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            candidates.push(d.join("models"));
            candidates.push(d.join("../Resources/models"));
            if let Some(name) = exe.file_stem() {
                candidates.push(d.join("../lib").join(name).join("models"));
            }
        }
    }
    candidates.into_iter().find(|d| d.join(LOCK_FILE).is_file())
}

/// Where a model loads from ([`locate`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The copy inside the installer (read-only).
    Bundled(PathBuf),
    /// The user's cache folder (downloads go here).
    User(PathBuf),
}

impl Source {
    /// The folder.
    pub fn dir(&self) -> &Path {
        match self {
            Self::Bundled(d) | Self::User(d) => d,
        }
    }

    /// Whether it is the installer's copy.
    pub fn is_bundled(&self) -> bool {
        matches!(self, Self::Bundled(_))
    }
}

/// A bundled model and whether this install has it ([`Resolver::bundled`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// The model's name (`whisper-base`).
    pub name: String,
    /// What it is for.
    pub kind: Kind,
    /// Its size in bytes.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
    /// Every file is inside this install (so nothing needs downloading).
    pub present: bool,
}

/// Finds models: the mode, the bundled folder and where verification markers go.
#[derive(Debug, Clone)]
pub struct Resolver {
    /// How models are got.
    pub mode: Mode,
    /// The bundled models folder, if this install has one.
    pub bundle: Option<PathBuf>,
    /// A writable folder for the "checked once" markers (`<data>/models`).
    pub state_dir: PathBuf,
}

impl Resolver {
    /// The resolver for this app: [`Mode::configured`] and [`bundle_dir`].
    pub fn new(state_dir: &Path) -> Self {
        Self { mode: Mode::configured(), bundle: bundle_dir(), state_dir: state_dir.to_path_buf() }
    }

    fn lock(&self) -> Result<Option<(PathBuf, Lock)>> {
        if self.mode == Mode::Download {
            return Ok(None);
        }
        match &self.bundle {
            Some(dir) => Ok(Some((dir.clone(), Lock::load(&dir.join(LOCK_FILE))?))),
            None => Ok(None),
        }
    }

    /// The folder `model` loads from: the bundled copy when it is there (and its checksums
    /// are right; checked once), else `user_dir`. With [`Mode::Bundle`] a model the lock
    /// lists but the install lacks is an error that says so.
    pub fn locate(&self, model: &str, user_dir: PathBuf) -> Result<Source> {
        let Some((dir, lock)) = self.lock()? else {
            if self.mode == Mode::Bundle {
                return Err(missing(format!("this app is built with its models inside, but its models folder ({LOCK_FILE}) is missing; reinstall the app")));
            }
            return Ok(Source::User(user_dir));
        };
        let files: Vec<&LockedFile> = lock.files_of(model).collect();
        if files.is_empty() {
            return Ok(Source::User(user_dir));
        }
        match self.check_once(&dir, model, &files) {
            Ok(()) => Ok(Source::Bundled(dir.join(model))),
            Err(why) if self.mode == Mode::Bundle => Err(missing(format!("the model {model} should be inside the app, but {why}; reinstall the app"))),
            Err(why) => {
                log::warn!("bundled model {model}: {why}; using the download instead");
                Ok(Source::User(user_dir))
            }
        }
    }

    /// Sizes and checksums of `model`'s files; the checksums only when the marker does not
    /// match (first start, or the files changed).
    fn check_once(&self, dir: &Path, model: &str, files: &[&LockedFile]) -> std::result::Result<(), String> {
        let mut stamp = format!("{}\n", dir.display());
        for f in files {
            let p = dir.join(model).join(&f.path);
            let m = std::fs::metadata(&p).map_err(|_| format!("{} is missing", f.path))?;
            if m.len() != f.size {
                return Err(format!("{} has {} bytes, not {}", f.path, m.len(), f.size));
            }
            let mtime = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
            stamp.push_str(&format!("{} {} {} {mtime}\n", f.path, f.size, f.sha256));
        }
        let marker = self.state_dir.join(".bundled").join(format!("{model}.ok"));
        if std::fs::read_to_string(&marker).is_ok_and(|s| s == stamp) {
            return Ok(());
        }
        for f in files {
            let sha = sha256_file(&dir.join(model).join(&f.path)).map_err(|e| e.to_string())?;
            if sha != f.sha256 {
                return Err(format!("{} failed its checksum", f.path));
            }
        }
        if let Err(e) = std::fs::create_dir_all(marker.parent().unwrap_or(&self.state_dir)).and_then(|_| std::fs::write(&marker, &stamp)) {
            log::warn!("could not record that {model} was checked: {e}");
        }
        Ok(())
    }

    /// The file of the first bundled model of `kind` (e.g. the GGUF), when it is there.
    pub fn bundled_file(&self, kind: Kind) -> Result<Option<PathBuf>> {
        let Some((_, lock)) = self.lock()? else { return Ok(None) };
        let Some(f) = lock.files.iter().find(|f| f.kind == kind) else { return Ok(None) };
        Ok(match self.locate(&f.model, PathBuf::new())? {
            Source::Bundled(d) => Some(d.join(&f.path)),
            Source::User(_) => None,
        })
    }

    /// The models this app bundles, and whether this install has each one (sizes only;
    /// for the UI, which skips their download steps).
    pub fn bundled(&self) -> Vec<ModelInfo> {
        let Ok(Some((dir, lock))) = self.lock() else { return Vec::new() };
        lock.models()
            .into_iter()
            .map(|m| {
                let files: Vec<&LockedFile> = lock.files_of(m).collect();
                ModelInfo {
                    name: m.to_string(),
                    kind: files[0].kind,
                    bytes: files.iter().map(|f| f.size).sum(),
                    present: files.iter().all(|f| check_file(&dir, f, false).is_none()),
                }
            })
            .collect()
    }
}

fn missing(msg: String) -> Error {
    Error::Io(std::io::Error::new(std::io::ErrorKind::NotFound, msg))
}

/// [`Resolver::locate`] for this app; `state_dir` is `<data>/models`.
pub fn locate(model: &str, user_dir: PathBuf, state_dir: &Path) -> Result<Source> {
    Resolver::new(state_dir).locate(model, user_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::Digest;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("models-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn line(model: &str, path: &str, data: &[u8]) -> String {
        format!("stt {model} {path} {} {:x} https://example.com/{path}\n", data.len(), sha2::Sha256::digest(data))
    }

    #[test]
    fn parses_and_refuses_bad_lines() {
        let l = Lock::parse(&format!("# c\n\n{}{}", line("whisper-base", "encoder.onnx", b"ab"), line("whisper-base", "vocab.json", b"c"))).unwrap();
        assert_eq!(l.models(), ["whisper-base"]);
        assert_eq!(l.bytes(), 3);
        for bad in ["stt a b", "xyz m f 1 00 u", &line("../x", "f", b""), &line("m", "../f", b""), &line("m", "/abs", b""), &line("m", "a\\b", b"")] {
            assert!(Lock::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn save_and_load_round_trip() {
        let d = tmp("rt");
        let l = Lock::parse(&line("m", "a/b.onnx", b"x")).unwrap();
        l.save(&d.join(LOCK_FILE)).unwrap();
        assert_eq!(Lock::load(&d.join(LOCK_FILE)).unwrap(), l);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn verify_finds_missing_wrong_size_and_wrong_checksum() {
        let d = tmp("verify");
        let l = Lock::parse(&format!("{}{}{}", line("m", "a", b"aaa"), line("m", "b", b"bbb"), line("m", "c", b"ccc"))).unwrap();
        std::fs::create_dir_all(d.join("m")).unwrap();
        std::fs::write(d.join("m/a"), b"aaa").unwrap();
        std::fs::write(d.join("m/b"), b"bb").unwrap();
        std::fs::write(d.join("m/c"), b"xxx").unwrap();
        let p: Vec<String> = verify(&l, &d, true).iter().map(|p| p.to_string()).collect();
        assert_eq!(p, ["m/b: 2 bytes, the lock says 3", "m/c: wrong checksum"]);
        assert_eq!(verify(&l, &d, false).len(), 1);
        std::fs::remove_file(d.join("m/a")).unwrap();
        assert_eq!(verify(&l, &d, false)[0].what, "missing");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn modes_decide_what_a_missing_bundled_model_means() {
        let d = tmp("modes");
        let bundle = d.join("bundle");
        Lock::parse(&line("whisper-base", "encoder.onnx", b"enc")).unwrap().save(&bundle.join(LOCK_FILE)).unwrap();
        let r = |mode| Resolver { mode, bundle: Some(bundle.clone()), state_dir: d.join("state") };
        let user = d.join("user");
        // Not fetched into the bundle: an error with `bundle`, the download with the fallback.
        let e = r(Mode::Bundle).locate("whisper-base", user.clone()).unwrap_err().to_string();
        assert!(e.contains("should be inside the app") && e.contains("encoder.onnx is missing"), "{e}");
        assert_eq!(r(Mode::BundleWithFallback).locate("whisper-base", user.clone()).unwrap(), Source::User(user.clone()));
        assert_eq!(r(Mode::Download).locate("whisper-base", user.clone()).unwrap(), Source::User(user.clone()));
        // No bundle folder at all.
        let none = Resolver { mode: Mode::Bundle, bundle: None, state_dir: d.join("state") };
        assert!(none.locate("whisper-base", user.clone()).unwrap_err().to_string().contains("reinstall"));
        // A corrupted file fails its checksum.
        std::fs::create_dir_all(bundle.join("whisper-base")).unwrap();
        std::fs::write(bundle.join("whisper-base/encoder.onnx"), b"bad").unwrap();
        assert!(r(Mode::Bundle).locate("whisper-base", user.clone()).unwrap_err().to_string().contains("checksum"));
        std::fs::write(bundle.join("whisper-base/encoder.onnx"), b"enc").unwrap();
        assert!(r(Mode::Bundle).locate("whisper-base", user.clone()).unwrap().is_bundled());
        assert!(d.join("state/.bundled/whisper-base.ok").is_file());
        assert_eq!(r(Mode::Bundle).bundled()[0], ModelInfo { name: "whisper-base".into(), kind: Kind::Stt, bytes: 3, present: true });
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hugging_face_refs_are_written() {
        let d = tmp("refs");
        let l = Lock::parse(&line("embed-x", "models--o--r/snapshots/abc123/onnx/model.onnx", b"m")).unwrap();
        write_hf_refs(&l, &d).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("embed-x/models--o--r/refs/main")).unwrap(), "abc123");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn modes_parse() {
        assert_eq!(Mode::parse("bundle_with_fallback"), Some(Mode::BundleWithFallback));
        assert_eq!(Mode::parse("bundle"), Some(Mode::Bundle));
        assert_eq!(Mode::parse("download"), Some(Mode::Download));
        assert_eq!(Mode::parse("sometimes"), None);
    }
}
