//! Search and RAG evals: score the app's search against a labelled dataset, and catch
//! regressions against a saved baseline (feature `evals`).
//!
//! A **dataset** is JSON lines, one case per line:
//!
//! ```text
//! {"query": "how much salt goes in pasta water", "relevant_doc_ids": ["pasta-water"], "expected_answer": "About ten grams per litre."}
//! ```
//!
//! `relevant_doc_ids` are document ids (the `source` of a [`Hit`]); `expected_answer` is
//! optional and only used by the answer path. A **corpus** is JSON lines of
//! `{"id": ..., "text": ...}` ([`Doc`]); [`ingest`] puts it into an [`Engine`] with each
//! id as the source. The template ships a small sample of both ([`SAMPLE_CORPUS`],
//! [`SAMPLE_DATASET`], from the app's `evals/` folder).
//!
//! For each search mode, [`run`] reports with binary relevance:
//!
//! - **recall@k**: the share of a query's relevant documents found in its top `k`,
//! - **MRR**: the mean of `1 / rank` of the first relevant document in the top `k` (0 when none is),
//! - **nDCG@k**: `sum(rel_i / log2(i + 1))` over the top `k`, divided by the same sum for a perfect ranking,
//! - **latency**: p50 and p95 per query (nearest-rank percentiles), and the mean, in milliseconds.
//!
//! [`evaluate`] runs every mode the engine can (keyword, which is BM25; with an embedder,
//! vector and hybrid too). [`Report::check`] compares a report with a saved one and fails
//! it when a quality metric dropped by more than [`Thresholds::max_drop`] (and, when set,
//! when latency grew past [`Thresholds::max_latency_ratio`]).
//!
//! The answer path is optional: [`run_answers`] asks a question for each case that has an
//! `expected_answer`, scores the reply's word overlap with it ([`token_f1`]) and, with a
//! [`Judge`], its faithfulness to the passages it was given. With feature `ai`,
//! `evaluate_answers` does this through the app's own `llm` module, and
//! `LlmJudge` grades through it too; both are skipped when AI is off or there is no API key.
//!
//! Entry points: `<app> --eval [dataset.jsonl] [--baseline file]` prints a JSON [`Report`],
//! `kitchen eval` wraps it, and `make eval` compares with `evals/baseline.json`.
//!
//! # Example
//!
//! Score a tiny in-memory BM25 search, then compare it with a baseline:
//!
//! ```
//! use desktop_core::eval::{self, Case, Report, Thresholds};
//! use desktop_core::store::bm25_scores;
//!
//! let docs = [("tea", "green tea steeps at 80 degrees"), ("bread", "bread dough rises in a warm place")];
//! let cases = eval::parse_dataset(r#"{"query": "why does dough rise", "relevant_doc_ids": ["bread"]}"#)?;
//!
//! // Any function from (query, k) to ranked document ids is a retriever.
//! let mut search = |query: &str, k: usize| {
//!     let scores = bm25_scores(query, docs.iter().map(|d| d.1));
//!     let mut ranked: Vec<(usize, f32)> = scores.into_iter().enumerate().filter(|s| s.1 > 0.0).collect();
//!     ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
//!     Ok(ranked.into_iter().take(k).map(|(i, _)| docs[i].0.to_string()).collect())
//! };
//! let run = eval::run(&cases, 5, &mut search)?;
//! assert_eq!((run.recall_at_k, run.mrr, run.ndcg_at_k), (1.0, 1.0, 1.0));
//!
//! let mut report = Report::new(5, docs.len());
//! report.runs.insert("keyword".into(), run);
//! let baseline = report.clone();
//! assert!(report.check(&baseline, &Thresholds::default()));
//! # Ok::<(), desktop_core::Error>(())
//! ```

use crate::{Engine, Error, Hit, Result, SearchConfig, SearchMode};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::time::Instant;

/// The sample corpus the template ships (`evals/corpus.jsonl`): short cooking notes.
pub const SAMPLE_CORPUS: &str = include_str!("../../evals/corpus.jsonl");

/// The sample dataset the template ships (`evals/dataset.jsonl`), labelled against [`SAMPLE_CORPUS`].
pub const SAMPLE_DATASET: &str = include_str!("../../evals/dataset.jsonl");

/// One labelled query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Case {
    /// What the user types.
    pub query: String,
    /// The ids of the documents that answer it (a [`Hit`]'s `source`).
    pub relevant_doc_ids: Vec<String>,
    /// A reference answer, for the answer path (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_answer: Option<String>,
}

/// One document of a corpus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Doc {
    /// Its id, stored as the chunk source.
    pub id: String,
    /// Its text.
    pub text: String,
}

/// Parse JSON lines (blank lines are skipped); an error names the line.
pub fn parse_jsonl<T: DeserializeOwned>(text: &str) -> Result<Vec<T>> {
    text.lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(n, l)| serde_json::from_str(l).map_err(|e| Error::Source(format!("line {}: {e}", n + 1))))
        .collect()
}

/// Parse a dataset; every case needs a query and at least one relevant id.
pub fn parse_dataset(text: &str) -> Result<Vec<Case>> {
    let cases: Vec<Case> = parse_jsonl(text)?;
    if let Some(c) = cases.iter().find(|c| c.query.trim().is_empty() || c.relevant_doc_ids.is_empty()) {
        return Err(Error::Source(format!("dataset case {:?}: needs a query and at least one relevant_doc_ids entry", c.query)));
    }
    if cases.is_empty() {
        return Err(Error::Source("the dataset has no cases".into()));
    }
    Ok(cases)
}

/// Parse a corpus; ids must be unique and not empty.
pub fn parse_corpus(text: &str) -> Result<Vec<Doc>> {
    let docs: Vec<Doc> = parse_jsonl(text)?;
    let mut seen = HashSet::new();
    for d in &docs {
        if d.id.trim().is_empty() || !seen.insert(d.id.as_str()) {
            return Err(Error::Source(format!("corpus id {:?} is empty or repeated", d.id)));
        }
    }
    Ok(docs)
}

/// The share of `relevant` found in the first `k` of `ranked` (0 when `relevant` is empty).
pub fn recall_at_k<S: AsRef<str>, R: AsRef<str>>(ranked: &[S], relevant: &[R], k: usize) -> f64 {
    let want: HashSet<&str> = relevant.iter().map(AsRef::as_ref).collect();
    if want.is_empty() {
        return 0.0;
    }
    let found: HashSet<&str> = ranked.iter().take(k).map(AsRef::as_ref).filter(|d| want.contains(d)).collect();
    found.len() as f64 / want.len() as f64
}

/// `1 / rank` of the first relevant document in the first `k` of `ranked`, or 0.
pub fn reciprocal_rank<S: AsRef<str>, R: AsRef<str>>(ranked: &[S], relevant: &[R], k: usize) -> f64 {
    let want: HashSet<&str> = relevant.iter().map(AsRef::as_ref).collect();
    ranked.iter().take(k).position(|d| want.contains(d.as_ref())).map_or(0.0, |i| 1.0 / (i + 1) as f64)
}

/// nDCG over the first `k` of `ranked`, with binary relevance (a repeated id counts once).
pub fn ndcg_at_k<S: AsRef<str>, R: AsRef<str>>(ranked: &[S], relevant: &[R], k: usize) -> f64 {
    let want: HashSet<&str> = relevant.iter().map(AsRef::as_ref).collect();
    let gain = |i: usize| 1.0 / ((i + 2) as f64).log2();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut dcg = 0.0;
    for (i, d) in ranked.iter().take(k).enumerate() {
        let d: &str = d.as_ref();
        if want.contains(d) && seen.insert(d) {
            dcg += gain(i);
        }
    }
    let ideal: f64 = (0..want.len().min(k)).map(gain).sum();
    if ideal == 0.0 {
        0.0
    } else {
        dcg / ideal
    }
}

/// The nearest-rank percentile `p` (0 to 100) of `samples`, or 0 for none.
pub fn percentile(samples: &[f64], p: f64) -> f64 {
    let mut s: Vec<f64> = samples.to_vec();
    s.sort_by(f64::total_cmp);
    if s.is_empty() {
        return 0.0;
    }
    let rank = ((p.clamp(0.0, 100.0) / 100.0) * s.len() as f64).ceil() as usize;
    s[rank.clamp(1, s.len()) - 1]
}

fn mean(xs: impl IntoIterator<Item = f64>) -> f64 {
    let (sum, n) = xs.into_iter().fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

/// Latency per query, in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Latency {
    /// The median.
    pub p50_ms: f64,
    /// The 95th percentile.
    pub p95_ms: f64,
    /// The mean.
    pub mean_ms: f64,
}

impl Latency {
    /// p50, p95 and the mean of `samples_ms`.
    pub fn of(samples_ms: &[f64]) -> Self {
        Self { p50_ms: percentile(samples_ms, 50.0), p95_ms: percentile(samples_ms, 95.0), mean_ms: mean(samples_ms.iter().copied()) }
    }
}

/// How one query did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueryResult {
    /// The query.
    pub query: String,
    /// The document ids returned, best first.
    pub retrieved: Vec<String>,
    /// recall@k for this query.
    pub recall: f64,
    /// `1 / rank` of the first relevant document, or 0.
    pub reciprocal_rank: f64,
    /// nDCG@k for this query.
    pub ndcg: f64,
    /// How long the search took.
    pub latency_ms: f64,
}

/// The scores of one search mode over a dataset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RunReport {
    /// Queries run.
    pub queries: usize,
    /// Mean recall@k.
    pub recall_at_k: f64,
    /// Mean reciprocal rank (within the top k).
    pub mrr: f64,
    /// Mean nDCG@k.
    pub ndcg_at_k: f64,
    /// Search latency.
    pub latency: Latency,
    /// Each query's result, in dataset order.
    pub per_query: Vec<QueryResult>,
}

/// Anything that ranks document ids for a query; closures `FnMut(&str, usize) -> Result<Vec<String>>` are retrievers.
pub trait Retriever {
    /// Up to `k` document ids for `query`, best first.
    fn retrieve(&mut self, query: &str, k: usize) -> Result<Vec<String>>;
}

impl<F: FnMut(&str, usize) -> Result<Vec<String>>> Retriever for F {
    fn retrieve(&mut self, query: &str, k: usize) -> Result<Vec<String>> {
        self(query, k)
    }
}

/// Run every case through `retriever` and score the top `k`.
pub fn run(cases: &[Case], k: usize, retriever: &mut dyn Retriever) -> Result<RunReport> {
    let k = k.max(1);
    let mut per_query = Vec::with_capacity(cases.len());
    for c in cases {
        let start = Instant::now();
        let retrieved = retriever.retrieve(&c.query, k)?;
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        per_query.push(QueryResult {
            query: c.query.clone(),
            recall: recall_at_k(&retrieved, &c.relevant_doc_ids, k),
            reciprocal_rank: reciprocal_rank(&retrieved, &c.relevant_doc_ids, k),
            ndcg: ndcg_at_k(&retrieved, &c.relevant_doc_ids, k),
            retrieved,
            latency_ms,
        });
    }
    let latencies: Vec<f64> = per_query.iter().map(|q| q.latency_ms).collect();
    Ok(RunReport {
        queries: per_query.len(),
        recall_at_k: mean(per_query.iter().map(|q| q.recall)),
        mrr: mean(per_query.iter().map(|q| q.reciprocal_rank)),
        ndcg_at_k: mean(per_query.iter().map(|q| q.ndcg)),
        latency: Latency::of(&latencies),
        per_query,
    })
}

/// An [`Engine`] search in one mode, as a [`Retriever`] of document ids (chunks of one
/// document count once, at the rank of its best chunk).
pub struct EngineRetriever<'a> {
    engine: &'a Engine,
    search: SearchConfig,
}

impl<'a> EngineRetriever<'a> {
    /// Search `engine` in `mode`, with the engine's other search settings.
    pub fn new(engine: &'a Engine, mode: SearchMode) -> Self {
        Self { engine, search: SearchConfig { mode, ..engine.config().search } }
    }
}

impl Retriever for EngineRetriever<'_> {
    fn retrieve(&mut self, query: &str, k: usize) -> Result<Vec<String>> {
        // Several chunks may come from one document: ask for more, then keep each id once.
        let cfg = SearchConfig { top_k: k.saturating_mul(4).max(k), ..self.search.clone() };
        Ok(doc_ids(&self.engine.search_with(query, &cfg)?, k))
    }
}

/// The distinct sources of `hits`, in order, at most `k`.
pub fn doc_ids(hits: &[Hit], k: usize) -> Vec<String> {
    let mut seen = HashSet::new();
    hits.iter().filter(|h| seen.insert(h.source.as_str())).take(k).map(|h| h.source.clone()).collect()
}

/// Put `docs` into `engine`, one source per document id. Returns the chunks added.
pub fn ingest(engine: &Engine, docs: &[Doc]) -> Result<usize> {
    docs.iter().map(|d| engine.ingest_text(&d.id, &d.text)).sum()
}

/// The modes worth scoring on `engine`: keyword always; vector and hybrid with an embedder.
pub fn modes(engine: &Engine) -> Result<Vec<SearchMode>> {
    Ok(if engine.status()?.embedder.is_some() {
        vec![SearchMode::Keyword, SearchMode::Vector, SearchMode::Hybrid]
    } else {
        vec![SearchMode::Keyword]
    })
}

/// The name a mode has in a report (`keyword`, `vector`, `hybrid`, `auto`).
pub fn mode_name(mode: SearchMode) -> String {
    serde_json::to_value(mode).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| format!("{mode:?}").to_lowercase())
}

/// Score `cases` on `engine` (already holding the corpus) in each of `modes` (empty: [`modes`]).
pub fn evaluate(engine: &Engine, cases: &[Case], k: usize, modes: &[SearchMode]) -> Result<Report> {
    let status = engine.status()?;
    let modes = if modes.is_empty() { self::modes(engine)? } else { modes.to_vec() };
    let mut report = Report::new(k.max(1), status.chunks);
    report.backend = status.backend;
    report.embedder = status.embedder;
    for mode in modes {
        let run = run(cases, report.k, &mut EngineRetriever::new(engine, mode))?;
        report.runs.insert(mode_name(mode), run);
    }
    Ok(report)
}

/// Grades how faithful an answer is to the passages it was given (an LLM judge, or a test stand-in).
pub trait Judge {
    /// A score from 0 (unsupported) to 1 (every claim supported by `sources`).
    fn faithfulness(&self, question: &str, answer: &str, sources: &[Hit], expected: Option<&str>) -> Result<f64>;
}

/// How one answer did.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnswerResult {
    /// The question.
    pub query: String,
    /// The reply (empty when it failed).
    pub answer: String,
    /// Word-overlap F1 with the expected answer.
    pub token_f1: f64,
    /// The judge's score, when there was a judge and it answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faithfulness: Option<f64>,
    /// How long the answer took (without the judge).
    pub latency_ms: f64,
    /// What went wrong, if anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The answer path's scores.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnswerReport {
    /// Questions asked (cases with an `expected_answer`).
    pub asked: usize,
    /// Answers the judge scored.
    pub judged: usize,
    /// Mean word-overlap F1 with the expected answers.
    pub token_f1: f64,
    /// Mean faithfulness, when anything was judged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faithfulness: Option<f64>,
    /// Answer latency.
    pub latency: Latency,
    /// Why the path (or its judge) did not run, e.g. no API key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
    /// Each answer, in dataset order.
    pub per_query: Vec<AnswerResult>,
}

impl AnswerReport {
    /// A report that says why nothing was asked.
    pub fn skipped(why: impl Into<String>) -> Self {
        Self { skipped: Some(why.into()), ..Self::default() }
    }
}

/// Lower-cased words of 2+ letters or digits, as search tokenises them.
fn words(text: &str) -> Vec<String> {
    crate::store::terms(text)
}

/// Word-overlap F1 between `answer` and `expected` (SQuAD style, counting repeats).
pub fn token_f1(answer: &str, expected: &str) -> f64 {
    let (a, e) = (words(answer), words(expected));
    if a.is_empty() || e.is_empty() {
        return 0.0;
    }
    let mut left: BTreeMap<&str, usize> = BTreeMap::new();
    for w in &e {
        *left.entry(w.as_str()).or_default() += 1;
    }
    let mut common = 0;
    for w in &a {
        if let Some(n) = left.get_mut(w.as_str()).filter(|n| **n > 0) {
            *n -= 1;
            common += 1;
        }
    }
    if common == 0 {
        return 0.0;
    }
    let (p, r) = (common as f64 / a.len() as f64, common as f64 / e.len() as f64);
    2.0 * p * r / (p + r)
}

/// Answers a question: the reply text and the passages it was given.
pub type Answerer<'a> = dyn FnMut(&str) -> Result<(String, Vec<Hit>)> + 'a;

/// Ask each case that has an `expected_answer` through `answer` (question to reply and
/// sources) and score it; `judge` (optional) grades faithfulness.
pub fn run_answers(cases: &[Case], answer: &mut Answerer<'_>, judge: Option<&dyn Judge>) -> AnswerReport {
    let mut per_query = Vec::new();
    for c in cases {
        let Some(expected) = c.expected_answer.as_deref() else { continue };
        let start = Instant::now();
        let result = answer(&c.query);
        let mut r = AnswerResult { query: c.query.clone(), latency_ms: start.elapsed().as_secs_f64() * 1000.0, ..AnswerResult::default() };
        match result {
            Ok((text, sources)) => {
                r.token_f1 = token_f1(&text, expected);
                if let Some(j) = judge {
                    match j.faithfulness(&c.query, &text, &sources, Some(expected)) {
                        Ok(s) => r.faithfulness = Some(s.clamp(0.0, 1.0)),
                        Err(e) => r.error = Some(format!("judge: {e}")),
                    }
                }
                r.answer = text;
            }
            Err(e) => r.error = Some(e.to_string()),
        }
        per_query.push(r);
    }
    let judged: Vec<f64> = per_query.iter().filter_map(|r| r.faithfulness).collect();
    let latencies: Vec<f64> = per_query.iter().map(|r| r.latency_ms).collect();
    AnswerReport {
        asked: per_query.len(),
        judged: judged.len(),
        token_f1: mean(per_query.iter().map(|r| r.token_f1)),
        faithfulness: (!judged.is_empty()).then(|| mean(judged.iter().copied())),
        latency: Latency::of(&latencies),
        skipped: None,
        per_query,
    }
}

/// The first number in `text` between 0 and 1 (a judge's reply such as `Score: 0.8`).
pub fn parse_score(text: &str) -> Option<f64> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter(|t| !t.is_empty())
        .filter_map(|t| t.trim_end_matches('.').parse::<f64>().ok())
        .find(|v| (0.0..=1.0).contains(v))
}

/// The instructions an LLM judge gets.
pub const JUDGE_PROMPT: &str = "You grade answers for faithfulness. Given numbered source passages, a question and an answer, \
    rate how well every claim in the answer is supported by the passages: 1 means fully supported, 0 means unsupported or contradicted. \
    Ignore style and length. Reply with one number between 0 and 1 and nothing else.";

{% if ai %}/// Why the AI answer path cannot run with `cfg` and `keys` (AI off, or no API key for a
/// cloud provider), or `None` when it can.
#[cfg(feature = "ai")]
pub fn llm_unavailable(cfg: &crate::llm::LlmConfig, keys: crate::llm::KeyLookup<'_>) -> Option<String> {
    let p = cfg.target.provider;
    if !cfg.enabled {
        Some("AI is off (llm.enabled is false)".into())
    } else if p.needs_key() && keys(p).is_none() {
        Some(format!("no API key for {} (set {} or add one in Settings)", p.id(), p.key_env().unwrap_or("a key")))
    } else {
        None
    }
}

/// A [`Judge`] that asks the configured model through [`crate::llm::Client`] (retries,
/// fallbacks, budgets and the network policy apply).
#[cfg(feature = "ai")]
pub struct LlmJudge<'a> {
    client: crate::llm::Client<'a>,
}

#[cfg(feature = "ai")]
impl<'a> LlmJudge<'a> {
    /// A judge on `cfg`; `usage_file` counts its tokens in the cost meter.
    pub fn new(cfg: crate::llm::LlmConfig, keys: crate::llm::KeyLookup<'a>, usage_file: Option<&std::path::Path>) -> Self {
        Self { client: crate::llm::Client::new(cfg, keys, usage_file) }
    }
}

#[cfg(feature = "ai")]
impl Judge for LlmJudge<'_> {
    fn faithfulness(&self, question: &str, answer: &str, sources: &[Hit], expected: Option<&str>) -> Result<f64> {
        use crate::llm::{Message, Role};
        let mut passages = String::new();
        for (i, h) in sources.iter().enumerate() {
            passages.push_str(&format!("[{}] {}\n\n", i + 1, h.text.trim()));
        }
        let reference = expected.map(|e| format!("\nReference answer (for context only): {e}\n")).unwrap_or_default();
        let messages = [
            Message::new(Role::System, JUDGE_PROMPT),
            Message::new(
                Role::User,
                format!("<sources>\n{passages}</sources>\n\nQuestion: {question}\n{reference}\nAnswer to grade:\n{answer}"),
            ),
        ];
        let reply = self.client.chat(&messages, &mut |_| {})?;
        parse_score(&reply.text).ok_or_else(|| Error::Llm(format!("the judge gave no score: {:?}", reply.text)))
    }
}

/// The answer path on `engine` through `Engine::ask`, judged by [`LlmJudge`] when `judge`
/// is true. Skipped, with the reason, when AI is off or there is no API key.
#[cfg(feature = "ai")]
pub fn evaluate_answers(engine: &Engine, cases: &[Case], keys: crate::llm::KeyLookup<'_>, judge: bool) -> AnswerReport {
    let cfg = engine.config().llm;
    if let Some(why) = llm_unavailable(&cfg, keys) {
        return AnswerReport::skipped(why);
    }
    let usage = engine.data_dir().join(crate::llm::USAGE_FILE);
    let grader = judge.then(|| LlmJudge::new(cfg, keys, Some(&usage)));
    let mut ask = |q: &str| engine.ask(q, keys, &mut |_| {}).map(|a| (a.reply.text, a.sources));
    let mut report = run_answers(cases, &mut ask, grader.as_ref().map(|j| j as &dyn Judge));
    if !judge {
        report.skipped = Some("the faithfulness judge was not asked for".into());
    }
    report
}

{% endif %}/// What counts as a regression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Thresholds {
    /// The largest drop allowed in recall@k, MRR, nDCG@k and faithfulness (absolute, 0 to 1).
    pub max_drop: f64,
    /// When set, the largest allowed ratio of p95 latency to the baseline's (e.g. 1.5).
    /// Off by default: latency depends on the machine.
    pub max_latency_ratio: Option<f64>,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self { max_drop: 0.02, max_latency_ratio: None }
    }
}

/// One metric that got worse than allowed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Regression {
    /// The run (`keyword`, `vector`, `hybrid`, `answers`).
    pub run: String,
    /// The metric (`recall_at_k`, `mrr`, `ndcg_at_k`, `p95_ms`, `faithfulness`).
    pub metric: String,
    /// The baseline's value.
    pub baseline: f64,
    /// This run's value.
    pub current: f64,
}

/// A report compared with a baseline.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Comparison {
    /// The thresholds used.
    pub thresholds: Option<Thresholds>,
    /// Metrics past their threshold.
    pub regressions: Vec<Regression>,
    /// Runs the baseline has and this report lacks (a mode that stopped working).
    pub missing: Vec<String>,
    /// Neither regressions nor missing runs.
    pub ok: bool,
}

/// The whole result of an eval: one run per search mode, the answer path, the baseline check.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// How many results each query was scored on.
    pub k: usize,
    /// Chunks in the store while it ran.
    pub chunks: usize,
    /// `file` or `postgres`.
    #[serde(default)]
    pub backend: String,
    /// The embedding model, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedder: Option<String>,
    /// One run per search mode, by mode name.
    pub runs: BTreeMap<String, RunReport>,
    /// The answer path, when it was asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<AnswerReport>,
    /// The comparison with a baseline, when one was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Comparison>,
    /// False when the baseline check failed.
    pub ok: bool,
}

impl Report {
    /// An empty report for top-`k` scoring over `chunks` chunks.
    pub fn new(k: usize, chunks: usize) -> Self {
        Self { k, chunks, ok: true, ..Self::default() }
    }

    /// Compare with `baseline` (a saved report), record the result and return whether it passed.
    pub fn check(&mut self, baseline: &Report, t: &Thresholds) -> bool {
        let c = compare(self, baseline, t);
        self.ok = c.ok;
        self.baseline = Some(c);
        self.ok
    }
}

/// `current` against `baseline` under `t`.
pub fn compare(current: &Report, baseline: &Report, t: &Thresholds) -> Comparison {
    let mut regressions = Vec::new();
    let mut missing = Vec::new();
    let mut drop = |run: &str, metric: &str, b: f64, c: f64| {
        if b - c > t.max_drop + 1e-9 {
            regressions.push(Regression { run: run.into(), metric: metric.into(), baseline: b, current: c });
        }
    };
    for (name, b) in &baseline.runs {
        let Some(c) = current.runs.get(name) else {
            missing.push(name.clone());
            continue;
        };
        drop(name, "recall_at_k", b.recall_at_k, c.recall_at_k);
        drop(name, "mrr", b.mrr, c.mrr);
        drop(name, "ndcg_at_k", b.ndcg_at_k, c.ndcg_at_k);
    }
    if let (Some(b), Some(c)) =
        (baseline.answers.as_ref().and_then(|a| a.faithfulness), current.answers.as_ref().and_then(|a| a.faithfulness))
    {
        drop("answers", "faithfulness", b, c);
    }
    if let Some(ratio) = t.max_latency_ratio {
        for (name, b) in &baseline.runs {
            if let Some(c) = current.runs.get(name) {
                if b.latency.p95_ms > 0.0 && c.latency.p95_ms > b.latency.p95_ms * ratio {
                    regressions.push(Regression {
                        run: name.clone(),
                        metric: "p95_ms".into(),
                        baseline: b.latency.p95_ms,
                        current: c.latency.p95_ms,
                    });
                }
            }
        }
    }
    let ok = regressions.is_empty() && missing.is_empty();
    Comparison { thresholds: Some(t.clone()), regressions, missing, ok }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-4, "{a} != {b}");
    }

    const RANKED: [&str; 5] = ["a", "b", "c", "d", "e"];
    const RELEVANT: [&str; 3] = ["b", "e", "x"];

    #[test]
    fn recall_by_hand() {
        // b and e of {b, e, x} in the top 5; only b in the top 3; nothing in the top 1.
        close(recall_at_k(&RANKED, &RELEVANT, 5), 2.0 / 3.0);
        close(recall_at_k(&RANKED, &RELEVANT, 3), 1.0 / 3.0);
        close(recall_at_k(&RANKED, &RELEVANT, 1), 0.0);
        close(recall_at_k(&RANKED, &[] as &[&str], 5), 0.0);
    }

    #[test]
    fn reciprocal_rank_by_hand() {
        close(reciprocal_rank(&RANKED, &RELEVANT, 5), 0.5); // b at rank 2
        close(reciprocal_rank(&RANKED, &["e"], 5), 0.2);
        close(reciprocal_rank(&RANKED, &["e"], 4), 0.0); // past k
        close(reciprocal_rank(&RANKED, &["a"], 5), 1.0);
    }

    #[test]
    fn ndcg_by_hand() {
        // DCG = 1/log2(3) + 1/log2(6) = 0.63093 + 0.38685 = 1.01778
        // IDCG (3 relevant) = 1 + 1/log2(3) + 1/log2(4) = 2.13093
        close(ndcg_at_k(&RANKED, &RELEVANT, 5), 1.01778 / 2.13093);
        // k = 3: DCG = 0.63093, IDCG = 2.13093
        close(ndcg_at_k(&RANKED, &RELEVANT, 3), 0.63093 / 2.13093);
        // k = 2, one relevant at rank 2: 0.63093 / 1
        close(ndcg_at_k(&RANKED, &["b"], 2), 0.63093);
        close(ndcg_at_k(&["b", "a"], &["b"], 2), 1.0);
        // A repeated id is not counted twice.
        close(ndcg_at_k(&["b", "b"], &["b", "c"], 2), 1.0 / 1.63093);
        close(ndcg_at_k(&RANKED, &["z"], 5), 0.0);
    }

    #[test]
    fn percentiles_by_hand() {
        let ten: Vec<f64> = (1..=10).map(f64::from).collect();
        close(percentile(&ten, 50.0), 5.0); // ceil(0.5 * 10) = 5th value
        close(percentile(&ten, 95.0), 10.0); // ceil(9.5) = 10th
        let four = [40.0, 10.0, 30.0, 20.0];
        close(percentile(&four, 50.0), 20.0);
        close(percentile(&four, 95.0), 40.0);
        close(percentile(&four, 0.0), 10.0);
        close(percentile(&[], 50.0), 0.0);
        let l = Latency::of(&four);
        close(l.mean_ms, 25.0);
    }

    #[test]
    fn token_f1_by_hand() {
        // answer: about ten grams (3), expected: about ten grams per litre (5); 3 shared.
        // P = 1, R = 0.6, F1 = 2 * 0.6 / 1.6 = 0.75
        close(token_f1("About ten grams", "about ten grams per litre"), 0.75);
        close(token_f1("nothing alike", "ten grams"), 0.0);
        close(token_f1("", "ten"), 0.0);
        // Repeats count up to how often they are expected: "ten ten" vs "ten": P = 0.5, R = 1.
        close(token_f1("ten ten", "ten"), 2.0 / 3.0);
    }

    #[test]
    fn run_averages_queries() {
        let cases = parse_dataset(
            "{\"query\": \"q1\", \"relevant_doc_ids\": [\"a\"]}\n\n{\"query\": \"q2\", \"relevant_doc_ids\": [\"z\", \"b\"]}\n",
        )
        .unwrap();
        let mut r = |q: &str, _k: usize| Ok(if q == "q1" { vec!["a".to_string()] } else { vec!["c".into(), "b".into()] });
        let run = run(&cases, 2, &mut r).unwrap();
        assert_eq!(run.queries, 2);
        close(run.recall_at_k, (1.0 + 0.5) / 2.0);
        close(run.mrr, (1.0 + 0.5) / 2.0);
        // q2: DCG = 0.63093, IDCG = 1 + 0.63093
        close(run.ndcg_at_k, (1.0 + 0.63093 / 1.63093) / 2.0);
    }

    #[test]
    fn bad_input_is_refused_with_the_line() {
        let e = parse_dataset("{\"query\": \"q\", \"relevant_doc_ids\": [\"a\"]}\nnot json").unwrap_err().to_string();
        assert!(e.contains("line 2"), "{e}");
        assert!(parse_dataset("{\"query\": \"q\", \"relevant_doc_ids\": []}").is_err());
        assert!(parse_dataset("").is_err());
        assert!(parse_corpus("{\"id\": \"a\", \"text\": \"x\"}\n{\"id\": \"a\", \"text\": \"y\"}").is_err());
    }

    #[test]
    fn the_sample_is_consistent() {
        let docs = parse_corpus(SAMPLE_CORPUS).unwrap();
        let cases = parse_dataset(SAMPLE_DATASET).unwrap();
        assert!(docs.len() >= 15 && cases.len() >= 10, "{} docs, {} cases", docs.len(), cases.len());
        let ids: HashSet<&str> = docs.iter().map(|d| d.id.as_str()).collect();
        for c in &cases {
            assert!(c.relevant_doc_ids.iter().all(|id| ids.contains(id.as_str())), "{:?} names an unknown doc", c.query);
        }
    }

    fn report(recall: f64, p95: f64) -> Report {
        let mut r = Report::new(5, 10);
        r.runs.insert(
            "keyword".into(),
            RunReport {
                recall_at_k: recall,
                mrr: 0.8,
                ndcg_at_k: 0.7,
                latency: Latency { p95_ms: p95, ..Latency::default() },
                ..RunReport::default()
            },
        );
        r
    }

    #[test]
    fn baseline_regressions() {
        let base = report(0.9, 10.0);
        // Within the threshold: passes.
        assert!(report(0.89, 50.0).check(&base, &Thresholds::default()));
        // Past it: fails and names the metric.
        let mut worse = report(0.8, 10.0);
        assert!(!worse.check(&base, &Thresholds::default()));
        let c = worse.baseline.unwrap();
        assert_eq!((c.regressions.len(), c.regressions[0].metric.as_str()), (1, "recall_at_k"));
        // Latency only when asked for.
        let slow = Thresholds { max_latency_ratio: Some(2.0), ..Thresholds::default() };
        assert!(!report(0.9, 25.0).check(&base, &slow));
        assert!(report(0.9, 15.0).check(&base, &slow));
        // A run the baseline has that is gone now fails; a new run does not.
        let mut gone = Report::new(5, 10);
        assert!(!gone.check(&base, &Thresholds::default()));
        assert_eq!(gone.baseline.unwrap().missing, ["keyword"]);
        let mut more = report(0.9, 10.0);
        more.runs.insert("vector".into(), RunReport::default());
        assert!(more.check(&base, &Thresholds::default()));
        // Reports round-trip through JSON (a saved baseline).
        let back: Report = serde_json::from_str(&serde_json::to_string(&more).unwrap()).unwrap();
        assert_eq!(back, more);
    }

    struct Fixed(f64);
    impl Judge for Fixed {
        fn faithfulness(&self, _q: &str, answer: &str, _s: &[Hit], _e: Option<&str>) -> Result<f64> {
            if answer.is_empty() {
                Err(Error::Llm("empty".into()))
            } else {
                Ok(self.0)
            }
        }
    }

    #[test]
    fn answers_are_scored_and_judged() {
        let cases = parse_dataset(concat!(
            "{\"query\": \"salt?\", \"relevant_doc_ids\": [\"a\"], \"expected_answer\": \"ten grams per litre\"}\n",
            "{\"query\": \"no reference\", \"relevant_doc_ids\": [\"a\"]}\n",
            "{\"query\": \"fails\", \"relevant_doc_ids\": [\"a\"], \"expected_answer\": \"x\"}\n",
        ))
        .unwrap();
        let mut answer =
            |q: &str| if q == "fails" { Err(Error::Llm("down".into())) } else { Ok(("ten grams per litre".to_string(), vec![])) };
        let r = run_answers(&cases, &mut answer, Some(&Fixed(0.75)));
        assert_eq!((r.asked, r.judged), (2, 1));
        close(r.faithfulness.unwrap(), 0.75);
        close(r.token_f1, 0.5); // 1.0 and 0.0
        assert!(r.per_query[1].error.as_deref().unwrap().contains("down"));
        let none = run_answers(&cases, &mut answer, None);
        assert_eq!((none.judged, none.faithfulness), (0, None));
    }

    #[test]
    fn judge_scores_are_parsed() {
        assert_eq!(parse_score("0.8"), Some(0.8));
        assert_eq!(parse_score("Score: 1."), Some(1.0));
        assert_eq!(parse_score("I would say 7 out of 10, so 0.7"), Some(0.7));
        assert_eq!(parse_score("no idea"), None);
    }

    #[test]
    fn engine_eval_on_the_sample() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::FileStore::open(dir.path()).unwrap();
        let engine = Engine::with_parts(Box::new(store), None, dir.path(), &mut |_| {}).unwrap();
        let docs = parse_corpus(SAMPLE_CORPUS).unwrap();
        assert!(ingest(&engine, &docs).unwrap() >= docs.len());
        let cases = parse_dataset(SAMPLE_DATASET).unwrap();
        let report = evaluate(&engine, &cases, 5, &[]).unwrap();
        assert_eq!(report.runs.keys().collect::<Vec<_>>(), ["keyword"]);
        let kw = &report.runs["keyword"];
        assert_eq!(kw.queries, cases.len());
        // Keyword search finds most of the sample; the baseline pins the exact numbers.
        assert!(kw.recall_at_k >= 0.6 && kw.mrr >= 0.5, "{kw:?}");
        assert!(kw.per_query.iter().all(|q| q.retrieved.len() <= 5));
    }
}
