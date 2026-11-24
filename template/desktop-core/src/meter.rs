//! Cost meter for language-model calls (feature `ai`): a price table per model, per-day and
//! per-month totals kept in the data folder, and budgets that warn and then block.
//!
//! - [`Price`] and [`default_prices`]: US dollars per million tokens for current Claude,
//!   OpenAI and Gemini models. **Edit me**: prices change, so the table lives in the config
//!   (`llm.prices`) where it can be corrected without a new build. A model matches its
//!   exact name, or else the longest name it starts with (`claude-sonnet-5-5-20261001`
//!   uses `claude-sonnet-5-5`).
//! - [`Usage`]: the tokens of one reply as the provider counted them, with the part read
//!   from (and written to) the provider's prompt cache. [`Usage::cost`] prices it.
//! - [`Ledger`]: totals per UTC day (`2026-11-24`) and month (`2026-11`), one JSON file
//!   (`llm-usage.json`, owner-only on Unix). [`Meter`] reads and adds to it.
//! - [`Limits`]: token and dollar budgets per day and month. [`Limits::check`] says
//!   whether a call may go ahead: `Warn` from `warn_percent` (80 by default), `Blocked` at
//!   100% unless the override is on.
//!
//! ```
//! use desktop_core::meter::{default_prices, price_for, Limits, Meter, State, Usage};
//!
//! let prices = default_prices();
//! let usage = Usage { input_tokens: 1_000_000, output_tokens: 100_000, ..Usage::default() };
//! let price = price_for(&prices, "claude-sonnet-5-5").unwrap();
//! assert!((usage.cost(price) - 3.0).abs() < 1e-9); // $2 in + $1 out
//!
//! let dir = tempfile::tempdir().unwrap();
//! let meter = Meter::new(dir.path().join("llm-usage.json"));
//! let day = 20_000; // days since 1970 (tests pass a fixed day)
//! meter.record_on(day, &usage, Some(3.0));
//! let limits = Limits { daily_usd: Some(3.5), warn_percent: 80, ..Limits::default() };
//! // $3.00 of $3.50 is past 80%: a warning, the call still goes ahead.
//! assert!(matches!(limits.check(&meter.ledger(), day), State::Warn(_)));
//! meter.record_on(day, &usage, Some(3.0));
//! assert!(matches!(limits.check(&meter.ledger(), day), State::Blocked(_)));
//! // The override lets calls through (with a warning).
//! let over = Limits { override_limits: true, ..limits };
//! assert!(matches!(over.check(&meter.ledger(), day), State::Warn(_)));
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What one model costs, in US dollars per million tokens.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Price {
    /// Prompt tokens.
    pub input: f64,
    /// Reply tokens.
    pub output: f64,
    /// Prompt tokens read from the provider's prompt cache (0: same as `input`).
    pub cached_input: f64,
    /// Prompt tokens written to the prompt cache (Anthropic; 0: same as `input`).
    pub cache_write: f64,
}

impl Price {
    /// A price with cache reads at `cached` and cache writes at the input price.
    pub const fn new(input: f64, output: f64, cached: f64, write: f64) -> Self {
        Self { input, output, cached_input: cached, cache_write: write }
    }
}

/// The built-in price table (USD per million tokens, list prices for the standard tier,
/// checked November 2026). **Edit me**: copy it into `llm.prices` and correct it when a
/// provider changes its prices or a new model comes out.
pub fn default_prices() -> BTreeMap<String, Price> {
    let table: &[(&str, Price)] = &[
        // Anthropic: cache writes (5-minute) cost 1.25x input.
        ("claude-fable-5-2", Price::new(8.0, 40.0, 0.20, 10.0)),
        ("claude-fable-5-1", Price::new(10.0, 50.0, 1.00, 12.5)),
        ("claude-opus-5-5", Price::new(4.0, 20.0, 0.20, 5.0)),
        ("claude-opus-5", Price::new(5.0, 25.0, 0.50, 6.25)),
        ("claude-opus-4", Price::new(5.0, 25.0, 0.50, 6.25)),
        ("claude-sonnet-5-5", Price::new(2.0, 10.0, 0.10, 2.5)),
        ("claude-sonnet-5", Price::new(2.0, 10.0, 0.20, 2.5)),
        ("claude-sonnet-4", Price::new(3.0, 15.0, 0.30, 3.75)),
        ("claude-haiku-5-5", Price::new(0.10, 0.50, 0.01, 0.125)),
        ("claude-haiku-4-5", Price::new(1.0, 5.0, 0.10, 1.25)),
        // OpenAI: cached input is read automatically; there is no write charge.
        ("gpt-5", Price::new(1.25, 10.0, 0.125, 0.0)),
        ("gpt-5-mini", Price::new(0.25, 2.0, 0.025, 0.0)),
        ("gpt-5-nano", Price::new(0.05, 0.40, 0.005, 0.0)),
        ("gpt-4.1", Price::new(2.0, 8.0, 0.50, 0.0)),
        ("gpt-4.1-mini", Price::new(0.40, 1.60, 0.10, 0.0)),
        ("gpt-4o", Price::new(2.50, 10.0, 1.25, 0.0)),
        ("gpt-4o-mini", Price::new(0.15, 0.60, 0.075, 0.0)),
        // Google Gemini (prompts up to 200k tokens).
        ("gemini-2.5-pro", Price::new(1.25, 10.0, 0.31, 0.0)),
        ("gemini-2.5-flash", Price::new(0.30, 2.50, 0.075, 0.0)),
        ("gemini-2.5-flash-lite", Price::new(0.10, 0.40, 0.025, 0.0)),
    ];
    table.iter().map(|(m, p)| (m.to_string(), *p)).collect()
}

/// The price for `model`: an exact entry, else the longest entry `model` starts with.
pub fn price_for<'a>(prices: &'a BTreeMap<String, Price>, model: &str) -> Option<&'a Price> {
    prices.get(model).or_else(|| prices.iter().filter(|(k, _)| !k.is_empty() && model.starts_with(k.as_str())).max_by_key(|(k, _)| k.len()).map(|(_, p)| p))
}

/// The tokens of one reply, as the provider counted them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Usage {
    /// Every prompt token, cached ones included.
    pub input_tokens: u64,
    /// Reply tokens.
    pub output_tokens: u64,
    /// Prompt tokens read from the prompt cache (part of `input_tokens`).
    pub cached_tokens: u64,
    /// Prompt tokens written to the prompt cache (part of `input_tokens`).
    pub cache_write_tokens: u64,
}

impl Usage {
    /// What these tokens cost at `price`, in US dollars.
    pub fn cost(&self, price: &Price) -> f64 {
        let or = |p: f64| if p > 0.0 { p } else { price.input };
        let fresh = self.input_tokens.saturating_sub(self.cached_tokens + self.cache_write_tokens);
        (fresh as f64 * price.input + self.cached_tokens as f64 * or(price.cached_input) + self.cache_write_tokens as f64 * or(price.cache_write) + self.output_tokens as f64 * price.output) / 1e6
    }
}

/// Totals for one day or month.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Totals {
    /// Answered calls.
    pub requests: u64,
    /// Calls whose model has no price (their cost is missing from `cost_usd`).
    pub unpriced: u64,
    /// The tokens.
    #[serde(flatten)]
    pub usage: Usage,
    /// What they cost, in US dollars.
    pub cost_usd: f64,
}

impl Totals {
    /// Prompt and reply tokens together (what the token budgets count).
    pub fn tokens(&self) -> u64 {
        self.usage.input_tokens + self.usage.output_tokens
    }

    fn add(&mut self, u: &Usage, cost: Option<f64>) {
        self.requests += 1;
        self.usage.input_tokens += u.input_tokens;
        self.usage.output_tokens += u.output_tokens;
        self.usage.cached_tokens += u.cached_tokens;
        self.usage.cache_write_tokens += u.cache_write_tokens;
        match cost {
            Some(c) => self.cost_usd += c,
            None => self.unpriced += 1,
        }
    }
}

/// Days of history kept in the ledger.
pub const KEEP_DAYS: usize = 92;
/// Months of history kept in the ledger.
pub const KEEP_MONTHS: usize = 36;

/// Totals per UTC day (`YYYY-MM-DD`) and month (`YYYY-MM`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ledger {
    /// Per day, oldest first.
    pub days: BTreeMap<String, Totals>,
    /// Per month, oldest first.
    pub months: BTreeMap<String, Totals>,
}

impl Ledger {
    /// The totals of `day` (days since 1970).
    pub fn day(&self, day: u64) -> Totals {
        self.days.get(&day_key(day)).cloned().unwrap_or_default()
    }

    /// The totals of the month `day` falls in.
    pub fn month(&self, day: u64) -> Totals {
        self.months.get(&month_key(day)).cloned().unwrap_or_default()
    }
}

/// Serialises read-modify-write of every ledger in this process.
static LOCK: Mutex<()> = Mutex::new(());

/// Reads and adds to the usage ledger file.
pub struct Meter(PathBuf);

impl Meter {
    /// The ledger at `file` (it need not exist yet).
    pub fn new(file: impl Into<PathBuf>) -> Self {
        Self(file.into())
    }

    /// The file.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// The ledger as stored (empty when missing or unreadable).
    pub fn ledger(&self) -> Ledger {
        std::fs::read(&self.0).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    /// Add one answered call today (UTC). `cost` is `None` when the model has no price.
    pub fn record(&self, usage: &Usage, cost: Option<f64>) -> Ledger {
        self.record_on(today(), usage, cost)
    }

    /// Add one answered call on `day` (days since 1970). A failure to write is logged.
    pub fn record_on(&self, day: u64, usage: &Usage, cost: Option<f64>) -> Ledger {
        let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut l = self.ledger();
        l.days.entry(day_key(day)).or_default().add(usage, cost);
        l.months.entry(month_key(day)).or_default().add(usage, cost);
        while l.days.len() > KEEP_DAYS {
            l.days.pop_first();
        }
        while l.months.len() > KEEP_MONTHS {
            l.months.pop_first();
        }
        if let Err(e) = self.save(&l) {
            log::warn!("AI usage not saved to {}: {e}", self.0.display());
        }
        l
    }

    fn save(&self, l: &Ledger) -> std::io::Result<()> {
        let tmp = self.0.with_extension("json.tmp");
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
        std::io::Write::write_all(&mut o.open(&tmp)?, &serde_json::to_vec_pretty(l)?)?;
        std::fs::rename(&tmp, &self.0)
    }
}

/// Budgets per day and month (`None`: no limit).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Limits {
    /// Tokens (in + out) per UTC day.
    pub daily_tokens: Option<u64>,
    /// Tokens (in + out) per UTC month.
    pub monthly_tokens: Option<u64>,
    /// US dollars per UTC day.
    pub daily_usd: Option<f64>,
    /// US dollars per UTC month.
    pub monthly_usd: Option<f64>,
    /// Warn from this share of a budget, in percent (0: never warn).
    pub warn_percent: u8,
    /// Keep calling after a budget is used up (it still warns).
    pub override_limits: bool,
}

/// Whether a call may go ahead under the budgets.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// Under every warning line.
    Ok,
    /// Past a warning line (or past a budget with the override on); the messages say which.
    Warn(Vec<String>),
    /// A budget is used up; the message says which and how to go on.
    Blocked(String),
}

/// One budget and how much of it is used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BudgetUse {
    /// `daily tokens`, `monthly tokens`, `daily USD` or `monthly USD`.
    pub name: String,
    /// Used so far.
    pub used: f64,
    /// The budget.
    pub limit: f64,
    /// `used / limit` in percent.
    pub percent: f64,
    /// `ok`, `warn` or `over`.
    pub state: String,
}

impl Limits {
    /// Every set budget and its use on `day`.
    pub fn uses(&self, ledger: &Ledger, day: u64) -> Vec<BudgetUse> {
        let (d, m) = (ledger.day(day), ledger.month(day));
        let rows = [
            ("daily tokens", d.tokens() as f64, self.daily_tokens.map(|v| v as f64)),
            ("monthly tokens", m.tokens() as f64, self.monthly_tokens.map(|v| v as f64)),
            ("daily USD", d.cost_usd, self.daily_usd),
            ("monthly USD", m.cost_usd, self.monthly_usd),
        ];
        rows.into_iter()
            .filter_map(|(name, used, limit)| {
                let limit = limit.filter(|l| l.is_finite() && *l >= 0.0)?;
                let percent = if limit > 0.0 { used / limit * 100.0 } else { 100.0 };
                let state = if percent >= 100.0 {
                    "over"
                } else if self.warn_percent > 0 && percent >= f64::from(self.warn_percent) {
                    "warn"
                } else {
                    "ok"
                };
                Some(BudgetUse { name: name.into(), used, limit, percent, state: state.into() })
            })
            .collect()
    }

    /// Whether a call may go ahead on `day`.
    pub fn check(&self, ledger: &Ledger, day: u64) -> State {
        let mut warns = Vec::new();
        for u in self.uses(ledger, day) {
            let (used, limit) = if u.name.ends_with("USD") { (format!("${:.2}", u.used), format!("${:.2}", u.limit)) } else { (format!("{}", u.used as u64), format!("{}", u.limit as u64)) };
            match u.state.as_str() {
                "over" if !self.override_limits => {
                    return State::Blocked(format!(
                        "the {} AI budget is used up ({used} of {limit}); raise it, wait for the next period, or turn on \"Keep going over budget\" in Settings → AI",
                        u.name
                    ))
                }
                "over" => warns.push(format!("over the {} AI budget ({used} of {limit}); the override is on", u.name)),
                "warn" => warns.push(format!("{:.0}% of the {} AI budget is used ({used} of {limit})", u.percent, u.name)),
                _ => {}
            }
        }
        if warns.is_empty() {
            State::Ok
        } else {
            State::Warn(warns)
        }
    }
}

/// What `ai_usage` shows: today, this month, recent history and the budgets.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Today (`YYYY-MM-DD`, UTC).
    pub day: String,
    /// This month (`YYYY-MM`).
    pub month: String,
    /// Today's totals.
    pub today: Totals,
    /// This month's totals.
    pub this_month: Totals,
    /// Up to the last 31 days with use, newest first.
    pub days: Vec<(String, Totals)>,
    /// Up to the last 12 months with use, newest first.
    pub months: Vec<(String, Totals)>,
    /// The set budgets and their use.
    pub budgets: Vec<BudgetUse>,
    /// Whether the override is on.
    pub override_limits: bool,
}

impl Report {
    /// The report for `day` from `ledger` under `limits`.
    pub fn new(ledger: &Ledger, limits: &Limits, day: u64) -> Self {
        Self {
            day: day_key(day),
            month: month_key(day),
            today: ledger.day(day),
            this_month: ledger.month(day),
            days: ledger.days.iter().rev().take(31).map(|(k, v)| (k.clone(), v.clone())).collect(),
            months: ledger.months.iter().rev().take(12).map(|(k, v)| (k.clone(), v.clone())).collect(),
            budgets: limits.uses(ledger, day),
            override_limits: limits.override_limits,
        }
    }
}

/// Today as days since 1970 (UTC).
pub fn today() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86_400).unwrap_or(0)
}

/// `YYYY-MM-DD` for a day number.
pub fn day_key(day: u64) -> String {
    let (y, m, d) = civil(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `YYYY-MM` for a day number.
pub fn month_key(day: u64) -> String {
    let (y, m, _) = civil(day);
    format!("{y:04}-{m:02}")
}

/// Year, month and day for days since 1970 (Howard Hinnant's days-to-civil algorithm).
fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_utc_calendar_days() {
        assert_eq!(day_key(0), "1970-01-01");
        assert_eq!(day_key(31), "1970-02-01");
        assert_eq!(month_key(365), "1971-01");
        assert_eq!(day_key(20_416), "2025-11-24");
        assert_eq!(day_key(11_016), "2000-02-29");
    }

    #[test]
    fn prices_match_exact_then_longest_prefix() {
        let p = default_prices();
        assert_eq!(price_for(&p, "gpt-5-mini").unwrap().input, 0.25);
        assert_eq!(price_for(&p, "gpt-5-mini-2026-01-01").unwrap().input, 0.25);
        assert_eq!(price_for(&p, "gpt-5").unwrap().input, 1.25);
        assert_eq!(price_for(&p, "claude-opus-4-8").unwrap().input, 5.0);
        assert!(price_for(&p, "llama3.2").is_none());
    }

    #[test]
    fn cost_splits_fresh_cached_and_written_tokens() {
        let price = Price::new(10.0, 20.0, 1.0, 12.5);
        let u = Usage { input_tokens: 1_000_000, output_tokens: 0, cached_tokens: 500_000, cache_write_tokens: 100_000 };
        // 400k fresh * 10 + 500k * 1 + 100k * 12.5 = 4 + 0.5 + 1.25
        assert!((u.cost(&price) - 5.75).abs() < 1e-9);
        // No cache prices: cached tokens cost the input price.
        assert!((u.cost(&Price::new(10.0, 0.0, 0.0, 0.0)) - 10.0).abs() < 1e-9);
    }

    #[test]
    fn ledger_keeps_days_and_months_and_survives_old_files() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("u.json");
        // The old format (one day's and month's token counts) reads as empty.
        std::fs::write(&f, r#"{"day":1,"month":0,"day_tokens":5,"month_tokens":5}"#).unwrap();
        let m = Meter::new(&f);
        assert_eq!(m.ledger(), Ledger::default());
        let u = Usage { input_tokens: 10, output_tokens: 5, ..Usage::default() };
        m.record_on(31, &u, Some(0.5));
        m.record_on(32, &u, None);
        let l = m.ledger();
        assert_eq!(l.day(31).tokens(), 15);
        assert_eq!(l.month(32).requests, 2);
        assert_eq!(l.month(32).unpriced, 1);
        assert!((l.month(32).cost_usd - 0.5).abs() < 1e-9);
        for day in 100..300 {
            m.record_on(day, &u, None);
        }
        assert_eq!(m.ledger().days.len(), KEEP_DAYS);
    }

    #[test]
    fn token_budgets_warn_then_block() {
        let mut l = Ledger::default();
        l.days.entry(day_key(5)).or_default().add(&Usage { input_tokens: 70, output_tokens: 10, ..Usage::default() }, None);
        let lim = Limits { daily_tokens: Some(100), warn_percent: 80, ..Limits::default() };
        assert!(matches!(lim.check(&l, 5), State::Warn(w) if w[0].contains("80%")));
        assert_eq!(lim.check(&l, 6), State::Ok);
        let lim = Limits { daily_tokens: Some(80), ..lim };
        assert!(matches!(lim.check(&l, 5), State::Blocked(m) if m.contains("daily tokens")));
        let r = Report::new(&l, &lim, 5);
        assert_eq!((r.budgets.len(), r.budgets[0].state.as_str(), r.today.requests), (1, "over", 1));
    }
}
