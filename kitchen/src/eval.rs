//! `kitchen eval`: run an app's search evals (template feature `evals`) from its folder.
//!
//! It builds the app with `cargo build -p <name>` (a no-op when nothing changed), runs
//! `<binary> --eval [dataset] [--baseline file] ...` and turns the app's JSON report into a
//! [`Report`]: one line per search mode, the regressions, and the whole JSON as the last
//! line (`report {...}`). A regression against the baseline is an error, so the command
//! exits non-zero and the MCP tool `eval` answers with `isError`. With `--dry-run` it only
//! says what it would run.
//!
//! ```
//! use kitchen::eval::{eval_args, summarize};
//! use serde_json::json;
//!
//! let args = eval_args(Some("evals/dataset.jsonl"), Some("evals/baseline.json"), Some("0.05"), None, false);
//! assert_eq!(args, ["--eval", "evals/dataset.jsonl", "--baseline", "evals/baseline.json", "--threshold", "0.05"]);
//!
//! let report = json!({"k": 5, "ok": true, "runs": {"keyword": {"recall_at_k": 0.9, "mrr": 0.8, "ndcg_at_k": 0.85,
//!     "latency": {"p50_ms": 0.4, "p95_ms": 1.0}}}});
//! let r = summarize(&report);
//! assert!(r.summary.starts_with("eval passed"));
//! assert!(r.lines[0].starts_with("keyword: recall@5 0.900"));
//! ```

use crate::project::Project;
use crate::{Error, Options, Report, Result};
use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// The arguments for the app binary.
pub fn eval_args(
    dataset: Option<&str>,
    baseline: Option<&str>,
    threshold: Option<&str>,
    k: Option<&str>,
    answers: bool,
) -> Vec<String> {
    let mut v = vec!["--eval".to_string()];
    v.extend(dataset.map(String::from));
    for (flag, value) in [
        ("--baseline", baseline),
        ("--threshold", threshold),
        ("--k", k),
    ] {
        if let Some(x) = value {
            v.extend([flag.to_string(), x.to_string()]);
        }
    }
    if answers {
        v.push("--answers".into());
    }
    v
}

fn num(v: &Value, path: &[&str]) -> f64 {
    path.iter()
        .try_fold(v, |v, k| v.get(k))
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
}

/// A readable report from the app's JSON report.
pub fn summarize(json: &Value) -> Report {
    let k = json.get("k").and_then(Value::as_u64).unwrap_or(0);
    let ok = json.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let mut lines = Vec::new();
    if let Some(runs) = json.get("runs").and_then(Value::as_object) {
        for (mode, r) in runs {
            lines.push(format!(
                "{mode}: recall@{k} {:.3}, MRR {:.3}, nDCG@{k} {:.3}, p50 {:.2} ms, p95 {:.2} ms",
                num(r, &["recall_at_k"]),
                num(r, &["mrr"]),
                num(r, &["ndcg_at_k"]),
                num(r, &["latency", "p50_ms"]),
                num(r, &["latency", "p95_ms"]),
            ));
        }
    }
    if let Some(a) = json.get("answers") {
        match a.get("skipped").and_then(Value::as_str) {
            Some(why) if a.get("asked").and_then(Value::as_u64) == Some(0) => {
                lines.push(format!("answers: skipped ({why})"))
            }
            _ => lines.push(format!(
                "answers: {} asked, token F1 {:.3}, faithfulness {}",
                a.get("asked").and_then(Value::as_u64).unwrap_or(0),
                num(a, &["token_f1"]),
                a.get("faithfulness")
                    .and_then(Value::as_f64)
                    .map_or("not judged".to_string(), |f| format!("{f:.3}")),
            )),
        }
    }
    let mut problems = 0;
    if let Some(b) = json.get("baseline") {
        for r in b
            .get("regressions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            problems += 1;
            lines.push(format!(
                "regression: {} {} {:.3} -> {:.3}",
                r.get("run").and_then(Value::as_str).unwrap_or("?"),
                r.get("metric").and_then(Value::as_str).unwrap_or("?"),
                num(r, &["baseline"]),
                num(r, &["current"]),
            ));
        }
        for m in b
            .get("missing")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            problems += 1;
            lines.push(format!(
                "regression: {} is in the baseline but did not run",
                m.as_str().unwrap_or("?")
            ));
        }
    }
    let summary = match (ok, json.get("baseline").is_some()) {
        (true, true) => "eval passed: no regression against the baseline".to_string(),
        (true, false) => "eval passed (no baseline given)".to_string(),
        (false, _) => format!("eval failed: {problems} regression(s) against the baseline"),
    };
    let mut r = Report::new(summary);
    r.lines = lines;
    r.lines.push(format!("report {json}"));
    r
}

/// Run the app's evals with `args` (from [`eval_args`]) for the app at `project`.
pub fn run(project: &Project, args: &[String], opts: &Options) -> Result<Report> {
    if !project.manifest.is_on("evals") {
        return Err(Error::Usage(
            "this app has no evals: turn them on with `kitchen enable evals`".into(),
        ));
    }
    let name = &project.manifest.app.name;
    let root = &project.root;
    if opts.dry_run {
        return Ok(Report::new(format!(
            "would run: cargo build -p {name}, then target/debug/{name} {}",
            args.join(" ")
        )));
    }
    let status = Command::new("cargo")
        .args(["build", "-p", name])
        .current_dir(root)
        .status()
        .map_err(|e| Error::Tool(format!("cargo build: {e}")))?;
    if !status.success() {
        return Err(Error::Tool(format!("cargo build -p {name} failed")));
    }
    let bin = crate::service::find_binary(root, name).ok_or_else(|| {
        Error::Tool(format!(
            "built, but found no binary for {name} under target/"
        ))
    })?;
    let out = Command::new(&bin)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| Error::Io(format!("run {}", bin.display()), e))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: Value = serde_json::from_str(stdout.trim()).map_err(|_| {
        let stderr = String::from_utf8_lossy(&out.stderr);
        Error::Tool(format!(
            "{} {} gave no report: {}",
            bin.display(),
            args.join(" "),
            stderr.trim()
        ))
    })?;
    let r = summarize(&json);
    if out.status.success() {
        Ok(r)
    } else {
        Err(Error::Tool(r.to_string()))
    }
}

/// Make a relative path from the command line absolute against `cwd` (the app runs in its root).
pub fn absolute(cwd: &Path, p: &str) -> String {
    let path = Path::new(p);
    if path.is_relative() {
        cwd.join(path).display().to_string()
    } else {
        p.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn regressions_fail_the_summary() {
        let j = json!({"k": 5, "ok": false, "runs": {}, "baseline": {
            "regressions": [{"run": "keyword", "metric": "mrr", "baseline": 0.9, "current": 0.5}],
            "missing": ["vector"], "ok": false}});
        let r = summarize(&j);
        assert!(r.summary.contains("2 regression(s)"), "{}", r.summary);
        assert!(r
            .lines
            .iter()
            .any(|l| l == "regression: keyword mrr 0.900 -> 0.500"));
        assert!(r.lines.last().unwrap().starts_with("report {"));
    }

    #[test]
    fn skipped_answers_say_why() {
        let j = json!({"k": 5, "ok": true, "runs": {}, "answers": {"asked": 0, "skipped": "no API key for anthropic"}});
        assert!(summarize(&j)
            .lines
            .iter()
            .any(|l| l == "answers: skipped (no API key for anthropic)"));
    }

    #[test]
    fn needs_the_evals_feature() {
        let dir = crate::TempDir::new("eval").unwrap();
        std::fs::write(
            dir.path().join("kitchen.toml"),
            "[app]\nname = \"x\"\n[answers]\nevals = false\n",
        )
        .unwrap();
        let p = Project::find(dir.path()).unwrap();
        let e = run(&p, &[], &Options::default()).unwrap_err().to_string();
        assert!(e.contains("kitchen enable evals"), "{e}");
        std::fs::write(
            dir.path().join("kitchen.toml"),
            "[app]\nname = \"x\"\n[answers]\nevals = true\n",
        )
        .unwrap();
        let p = Project::find(dir.path()).unwrap();
        let dry = Options {
            dry_run: true,
            ..Options::default()
        };
        let r = run(&p, &eval_args(None, None, None, None, true), &dry).unwrap();
        assert!(r.summary.contains("--eval --answers"), "{}", r.summary);
    }
}
