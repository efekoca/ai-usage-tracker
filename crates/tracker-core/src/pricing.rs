//! API-equivalent cost calculation driven by an editable `pricing.json`.
//! Unknown models are never priced by guesswork: they are reported as unpriced.

use crate::model::{Provider, Tokens};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The pricing file shipped with the app (users can override it with their own copy).
pub const DEFAULT_PRICING_JSON: &str = include_str!("../../../config/pricing.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingFile {
    pub schema_version: u32,
    pub updated_at: String,
    #[serde(default = "usd")]
    pub currency: String,
    #[serde(default)]
    pub sources: HashMap<String, String>,
    #[serde(default)]
    pub notes: Vec<String>,
    pub models: Vec<ModelPrice>,
    /// User-defined "price model X like model Y" mappings.
    #[serde(default)]
    pub user_aliases: HashMap<String, String>,
}

fn usd() -> String {
    "USD".into()
}

/// USD per million tokens.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Rates {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: Option<f64>,
    #[serde(default)]
    pub cache_write_5m: Option<f64>,
    #[serde(default)]
    pub cache_write_1h: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongContext {
    pub threshold: u64,
    #[serde(flatten)]
    pub rates: Rates,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPrice {
    pub id: String,
    pub provider: Provider,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(flatten)]
    pub rates: Rates,
    #[serde(default)]
    pub long_context: Option<LongContext>,
    #[serde(default)]
    pub speed_multipliers: HashMap<String, f64>,
    #[serde(default)]
    pub geo_multipliers: HashMap<String, f64>,
    #[serde(default)]
    pub web_search_per_1k: Option<f64>,
    #[serde(default)]
    pub verified_at: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

/// Cost of some usage, split by token category.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    pub web_search: f64,
}

impl Cost {
    pub fn total(&self) -> f64 {
        self.input + self.output + self.cache_read + self.cache_write + self.web_search
    }
    pub fn add(&mut self, o: &Cost) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.web_search += o.web_search;
    }
}

/// What the calculator needs from one usage record.
pub struct CostInput<'a> {
    pub model: &'a str,
    pub tokens: &'a Tokens,
    pub request_input: u64,
    pub web_search_requests: u32,
    pub speed: Option<&'a str>,
    pub inference_geo: Option<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum PricingError {
    #[error("invalid pricing file: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("unsupported pricing schema version {0}")]
    Schema(u32),
}

pub struct PriceBook {
    file: PricingFile,
    index: HashMap<String, usize>,
}

impl PriceBook {
    pub fn default_book() -> PriceBook {
        PriceBook::from_json(DEFAULT_PRICING_JSON).expect("bundled pricing.json is valid")
    }

    pub fn from_json(s: &str) -> Result<PriceBook, PricingError> {
        let file: PricingFile = serde_json::from_str(s)?;
        if file.schema_version != 1 {
            return Err(PricingError::Schema(file.schema_version));
        }
        let mut index = HashMap::new();
        for (i, m) in file.models.iter().enumerate() {
            index.insert(m.id.to_ascii_lowercase(), i);
            for a in &m.aliases {
                index.insert(a.to_ascii_lowercase(), i);
            }
        }
        Ok(PriceBook { file, index })
    }

    pub fn file(&self) -> &PricingFile {
        &self.file
    }

    /// Exact id → alias → user alias → id without a `-YYYYMMDD` date suffix. Nothing fuzzier,
    /// so a new model is reported as unpriced instead of silently getting a wrong price.
    pub fn lookup(&self, model: &str) -> Option<&ModelPrice> {
        let m = model.trim().to_ascii_lowercase();
        let find = |k: &str| self.index.get(k).map(|&i| &self.file.models[i]);
        find(&m)
            .or_else(|| {
                self.file
                    .user_aliases
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(&m))
                    .and_then(|(_, target)| find(&target.to_ascii_lowercase()))
            })
            .or_else(|| strip_date_suffix(&m).and_then(find))
    }

    /// `None` when the model has no known price.
    pub fn cost(&self, c: &CostInput) -> Option<Cost> {
        let p = self.lookup(c.model)?;
        let rates = match &p.long_context {
            Some(lc) if c.request_input > lc.threshold => &lc.rates,
            _ => &p.rates,
        };
        let mult = c.speed.and_then(|s| p.speed_multipliers.get(s)).copied().unwrap_or(1.0)
            * c.inference_geo.and_then(|g| p.geo_multipliers.get(g)).copied().unwrap_or(1.0);
        let per = |tokens: u64, rate: f64| tokens as f64 * rate / 1_000_000.0 * mult;
        let cw5_rate = rates.cache_write_5m.unwrap_or(rates.input);
        let cw1_rate = rates.cache_write_1h.unwrap_or(cw5_rate);
        let t = c.tokens;
        let cw_1h = t.cache_write_1h.min(t.cache_write);
        Some(Cost {
            input: per(t.input, rates.input),
            output: per(t.output, rates.output),
            cache_read: per(t.cache_read, rates.cache_read.unwrap_or(rates.input)),
            cache_write: per(t.cache_write - cw_1h, cw5_rate) + per(cw_1h, cw1_rate),
            web_search: p.web_search_per_1k.map(|r| c.web_search_requests as f64 * r / 1000.0).unwrap_or(0.0),
        })
    }
}

fn strip_date_suffix(m: &str) -> Option<&str> {
    let (head, tail) = m.rsplit_once('-')?;
    (tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit())).then_some(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(input: u64, cr: u64, cw: u64, cw1h: u64, out: u64) -> Tokens {
        Tokens { input, cache_read: cr, cache_write: cw, cache_write_1h: cw1h, output: out, reasoning: 0 }
    }

    fn cost(book: &PriceBook, model: &str, t: &Tokens, req: u64, speed: Option<&str>, geo: Option<&str>) -> Option<f64> {
        book.cost(&CostInput { model, tokens: t, request_input: req, web_search_requests: 0, speed, inference_geo: geo })
            .map(|c| (c.total() * 1e6).round() / 1e6)
    }

    #[test]
    fn bundled_file_parses_and_every_model_has_a_verification_date() {
        let b = PriceBook::default_book();
        assert!(b.file().models.len() > 10);
        assert!(b.file().models.iter().all(|m| m.verified_at.is_some() && m.rates.input > 0.0 && m.rates.output > 0.0));
    }

    #[test]
    fn anthropic_cache_ttl_tiers_are_priced_separately() {
        let b = PriceBook::default_book();
        // opus 5.5: 1M input $4, 1M 5m-write $5, 1M 1h-write $8, 1M read $0.20, 1M output $20
        let t = tokens(1_000_000, 1_000_000, 2_000_000, 1_000_000, 1_000_000);
        assert_eq!(cost(&b, "claude-opus-5-5", &t, 0, None, None), Some(4.0 + 0.2 + 5.0 + 8.0 + 20.0));
    }

    #[test]
    fn fast_mode_and_us_residency_multiply() {
        let b = PriceBook::default_book();
        let t = tokens(1_000_000, 0, 0, 0, 0);
        assert_eq!(cost(&b, "claude-opus-5-5", &t, 0, Some("fast"), None), Some(8.0));
        assert_eq!(cost(&b, "claude-opus-5-5", &t, 0, Some("standard"), Some("us")), Some(4.4));
        assert_eq!(cost(&b, "claude-opus-5-5", &t, 0, None, Some("not_available")), Some(4.0));
    }

    #[test]
    fn openai_long_context_tier_is_chosen_per_request() {
        let b = PriceBook::default_book();
        let t = tokens(1_000_000, 0, 0, 0, 0);
        assert_eq!(cost(&b, "gpt-5.6-sol", &t, 272_000, None, None), Some(4.0));
        assert_eq!(cost(&b, "gpt-5.6-sol", &t, 272_001, None, None), Some(8.0));
    }

    #[test]
    fn missing_cache_write_rate_falls_back_to_input() {
        let b = PriceBook::default_book();
        let t = tokens(0, 0, 1_000_000, 0, 0);
        assert_eq!(cost(&b, "gpt-5.5", &t, 0, None, None), Some(5.0));
    }

    #[test]
    fn dated_ids_and_aliases_resolve_but_unknown_models_do_not() {
        let b = PriceBook::default_book();
        assert_eq!(b.lookup("claude-haiku-4-5-20251001").unwrap().id, "claude-haiku-4-5");
        assert_eq!(b.lookup("claude-sonnet-4-5-20250929").unwrap().id, "claude-sonnet-4-5");
        assert_eq!(b.lookup("CLAUDE-OPUS-5").unwrap().id, "claude-opus-5");
        assert!(b.lookup("codex-auto-review").is_none());
        assert!(b.lookup("claude-opus-5-5-preview").is_none()); // no fuzzy matching
        assert!(b.lookup("gpt-5.6").is_none());
    }

    #[test]
    fn user_alias_prices_an_unknown_model_like_a_known_one() {
        let mut f: PricingFile = serde_json::from_str(DEFAULT_PRICING_JSON).unwrap();
        f.user_aliases.insert("codex-auto-review".into(), "gpt-5.6-terra".into());
        let b = PriceBook::from_json(&serde_json::to_string(&f).unwrap()).unwrap();
        assert_eq!(b.lookup("codex-auto-review").unwrap().id, "gpt-5.6-terra");
    }

    #[test]
    fn web_search_fee_is_per_request() {
        let b = PriceBook::default_book();
        let t = Tokens::default();
        let c = b
            .cost(&CostInput { model: "claude-sonnet-5", tokens: &t, request_input: 0, web_search_requests: 3, speed: None, inference_geo: None })
            .unwrap();
        assert!((c.web_search - 0.03).abs() < 1e-12);
    }

    #[test]
    fn rejects_unknown_schema() {
        assert!(PriceBook::from_json(r#"{"schema_version":2,"updated_at":"x","models":[]}"#).is_err());
    }
}
