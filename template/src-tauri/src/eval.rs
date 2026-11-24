//! `<app> --eval`: score this build's search (and, optionally, AI answers) on a labelled
//! dataset and print a JSON report (feature `evals`; the logic is `desktop_core::eval`).
//!
//! ```text
//! my-app --eval                                   the bundled sample (evals/ in the source)
//! my-app --eval evals/dataset.jsonl               a dataset; the corpus is corpus.jsonl next to it
//!        [--corpus docs.jsonl] [--k 5]            another corpus; results scored per query
//!        [--baseline evals/baseline.json]         fail (exit 1) on a regression against it
//!        [--threshold 0.02] [--latency-ratio 1.5] how much worse counts as a regression
//!        [--out report.json]                      also write the report to a file
//!        [--answers] [--no-judge]                 the AI answer path (feature ai), judged unless --no-judge
//!        [--backend file|postgres]                the store to index into (a fresh, temporary one)
//! ```
//!
//! The search settings and the AI settings are the saved ones (`settings.json`), so the
//! report measures the app as it is configured; the index is a temporary one. Exit codes:
//! 0 passed, 1 a regression against the baseline, 2 bad arguments or an error.

use desktop_core::eval::{self, Report, Thresholds};
use desktop_core::{Backend, Engine, EngineOptions};
use std::path::{Path, PathBuf};

/// What the command line asked for.
#[derive(Debug, Default, PartialEq)]
struct Args {
    dataset: Option<PathBuf>,
    corpus: Option<PathBuf>,
    baseline: Option<PathBuf>,
    out: Option<PathBuf>,
    k: Option<usize>,
    threshold: Option<f64>,
    latency_ratio: Option<f64>,
    answers: bool,
    no_judge: bool,
    backend: Option<String>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = args.iter();
    while let Some(w) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or_else(|| format!("{name} needs a value"));
        match w.as_str() {
            "--corpus" => a.corpus = Some(value(w)?.into()),
            "--baseline" => a.baseline = Some(value(w)?.into()),
            "--out" => a.out = Some(value(w)?.into()),
            "--k" => a.k = Some(value(w)?.parse().map_err(|e| format!("--k: {e}"))?),
            "--threshold" => a.threshold = Some(value(w)?.parse().map_err(|e| format!("--threshold: {e}"))?),
            "--latency-ratio" => a.latency_ratio = Some(value(w)?.parse().map_err(|e| format!("--latency-ratio: {e}"))?),
            "--backend" => a.backend = Some(value(w)?),
            "--answers" => a.answers = true,
            "--no-judge" => a.no_judge = true,
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            s if a.dataset.is_none() => a.dataset = Some(s.into()),
            s => return Err(format!("unexpected argument {s}")),
        }
    }
    Ok(a)
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Run `--eval` with the arguments after it; returns the exit code.
pub fn main(args: &[String]) -> i32 {
    match eval_cli(args) {
        Ok(ok) => i32::from(!ok),
        Err(e) => {
            eprintln!("eval: {e}");
            2
        }
    }
}

fn eval_cli(args: &[String]) -> Result<bool, String> {
    let a = parse(args)?;
    let (dataset, corpus) = match &a.dataset {
        Some(d) => {
            let corpus = a.corpus.clone().unwrap_or_else(|| d.parent().unwrap_or(Path::new(".")).join("corpus.jsonl"));
            (read(d)?, read(&corpus)?)
        }
        None => {
            (eval::SAMPLE_DATASET.to_string(), a.corpus.as_deref().map(read).transpose()?.unwrap_or_else(|| eval::SAMPLE_CORPUS.into()))
        }
    };
    let cases = eval::parse_dataset(&dataset).map_err(|e| format!("dataset: {e}"))?;
    let docs = eval::parse_corpus(&corpus).map_err(|e| format!("corpus: {e}"))?;
    let baseline: Option<Report> = match &a.baseline {
        Some(b) => Some(serde_json::from_str(&read(b)?).map_err(|e| format!("{}: {e}", b.display()))?),
        None => None,
    };
    let backend = match a.backend.as_deref() {
        None | Some("file") => Backend::File,
        Some("postgres") => Backend::Postgres,
        Some(other) => return Err(format!("--backend {other}: use file or postgres")),
    };

    let (config_dir, data_dir) = super::paths::app_dirs();
    let settings = super::settings_at(&config_dir.join("settings.json"));
    let tmp = std::env::temp_dir().join(format!("app-eval-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let opts = EngineOptions { data_dir: tmp.clone(), backend, embeddings: true, config: settings.config, app_name: None };
    let result = (|| -> Result<Report, String> {
        let engine = Engine::open(&opts, &mut |_| {}).map_err(|e| e.to_string())?;
        eval::ingest(&engine, &docs).map_err(|e| e.to_string())?;
        let mut report = eval::evaluate(&engine, &cases, a.k.unwrap_or(5), &[]).map_err(|e| e.to_string())?;
        if a.answers {
            report.answers = Some(answers(&engine, &cases, &data_dir, !a.no_judge));
        }
        Ok(report)
    })();
    #[cfg(feature = "postgres")]
    if backend == Backend::Postgres {
        desktop_core::pgstore::shutdown();
    }
    let _ = std::fs::remove_dir_all(&tmp);
    let mut report = result?;
    if let Some(b) = &baseline {
        let defaults = Thresholds::default();
        let t = Thresholds {
            max_drop: a.threshold.unwrap_or(defaults.max_drop),
            max_latency_ratio: a.latency_ratio.or(defaults.max_latency_ratio),
        };
        report.check(b, &t);
    }
    let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
    println!("{text}");
    if let Some(out) = &a.out {
        std::fs::write(out, &text).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    Ok(report.ok)
}

/// The AI answer path, with the API keys of the app's real data folder.
#[cfg(feature = "ai")]
fn answers(engine: &Engine, cases: &[eval::Case], data_dir: &Path, judge: bool) -> eval::AnswerReport {
    let keys = desktop_core::llm::KeyFile::new(data_dir.join("keys.json"));
    let lookup = |p| keys.get(p);
    eval::evaluate_answers(engine, cases, &lookup, judge)
}

#[cfg(not(feature = "ai"))]
fn answers(_engine: &Engine, _cases: &[eval::Case], _data_dir: &Path, _judge: bool) -> eval::AnswerReport {
    eval::AnswerReport::skipped("this build has no AI (feature ai)")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses_the_command_line() {
        let a = parse(&words("data.jsonl --baseline b.json --k 3 --threshold 0.05 --answers")).unwrap();
        assert_eq!(a.dataset.as_deref(), Some(Path::new("data.jsonl")));
        assert_eq!(a.baseline.as_deref(), Some(Path::new("b.json")));
        assert_eq!((a.k, a.threshold, a.answers, a.no_judge), (Some(3), Some(0.05), true, false));
        assert_eq!(parse(&[]).unwrap(), Args::default());
        assert!(parse(&words("--k")).is_err());
        assert!(parse(&words("--k x")).is_err());
        assert!(parse(&words("--nope")).is_err());
        assert!(parse(&words("a.jsonl b.jsonl")).is_err());
    }
}
