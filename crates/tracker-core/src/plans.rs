//! Plan list (`config/plans.json`): list prices, ladder order and published usage ratios.

use crate::model::Provider;
use serde::Deserialize;
use std::collections::HashMap;

pub const DEFAULT_PLANS_JSON: &str = include_str!("../../../config/plans.json");

#[derive(Debug, Clone, Deserialize)]
pub struct PlanDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub windows: Vec<String>,
    #[serde(default)]
    pub monthly_usd: Option<f64>,
    /// Position among the provider's personal plans (higher = more usage). Team, enterprise
    /// and API plans have none: no recommendation is made for them.
    #[serde(default)]
    pub ladder: Option<u32>,
    /// Per-session (five-hour) allowance relative to the provider's base plan, as published.
    #[serde(default)]
    pub session_multiple: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProviderPlans {
    plans: Vec<PlanDef>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlansFile {
    providers: HashMap<String, ProviderPlans>,
}

impl PlansFile {
    pub fn bundled() -> PlansFile {
        serde_json::from_str(DEFAULT_PLANS_JSON).expect("bundled plans.json is valid")
    }

    pub fn plans(&self, p: Provider) -> &[PlanDef] {
        self.providers.get(p.as_str()).map(|x| x.plans.as_slice()).unwrap_or(&[])
    }

    pub fn find(&self, p: Provider, id: &str) -> Option<&PlanDef> {
        self.plans(p).iter().find(|x| x.id == id)
    }

    pub fn above(&self, p: Provider, plan: &PlanDef) -> Option<&PlanDef> {
        let rung = plan.ladder?;
        self.plans(p).iter().filter(|x| x.ladder.is_some_and(|l| l > rung)).min_by_key(|x| x.ladder)
    }

    pub fn below(&self, p: Provider, plan: &PlanDef) -> Option<&PlanDef> {
        let rung = plan.ladder?;
        self.plans(p).iter().filter(|x| x.ladder.is_some_and(|l| l < rung)).max_by_key(|x| x.ladder)
    }

    /// One of several accounts' plan: the one chosen for it, else the one its newest reading
    /// names, else the provider's. A reading that fits more than one plan (Claude's "max") takes
    /// the provider's plan only when that is one of them; a reading no listed plan fits rules the
    /// provider's plan out too. Either way the choice is left to the user.
    pub fn account_plan(&self, p: Provider, chosen: Option<&str>, read: Option<&str>, provider_plan: Option<&str>) -> Option<String> {
        if let Some(id) = chosen.filter(|id| self.find(p, id).is_some()) {
            return Some(id.to_owned());
        }
        let provider_plan = provider_plan.filter(|id| self.find(p, id).is_some());
        let Some(read) = read else { return provider_plan.map(str::to_owned) };
        let fit: Vec<&str> = self.plans(p).iter().map(|d| d.id.as_str()).filter(|id| fits(p, id, read)).collect();
        match (fit.as_slice(), provider_plan) {
            (_, Some(plan)) if fit.contains(&plan) => Some(plan.to_owned()),
            ([only], _) => Some((*only).to_owned()),
            _ => None,
        }
    }
}

/// Whether a limit reading's plan name fits a plan: Claude reports "max" for both Max plans,
/// Codex "team" for Business and "edu" for Enterprise / Edu.
pub fn fits(p: Provider, id: &str, read: &str) -> bool {
    match (p, id) {
        (Provider::Anthropic, id) if id.starts_with("max") => read == "max",
        (Provider::OpenAI, "business") => read == "business" || read == "team",
        (Provider::OpenAI, "enterprise") => read == "enterprise" || read == "edu",
        _ => id == read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_plans_form_a_ladder_with_published_ratios() {
        let f = PlansFile::bundled();
        let max20 = f.find(Provider::Anthropic, "max20x").unwrap();
        let max5 = f.below(Provider::Anthropic, max20).unwrap();
        assert_eq!(max5.id, "max5x");
        assert_eq!((max20.session_multiple, max5.session_multiple), (Some(20.0), Some(5.0)));
        assert!(f.above(Provider::Anthropic, max20).is_none());
        let team = f.find(Provider::Anthropic, "team_premium").unwrap();
        assert!(f.above(Provider::Anthropic, team).is_none() && f.below(Provider::Anthropic, team).is_none());
        let plus = f.find(Provider::OpenAI, "plus").unwrap();
        let pro = f.above(Provider::OpenAI, plus).unwrap();
        assert_eq!(pro.id, "pro");
        assert!(!pro.windows.iter().any(|w| w == "five_hour"), "Pro has no five-hour limit");
        assert!(f.plans(Provider::OpenAI).iter().all(|p| p.session_multiple.is_none()), "OpenAI publishes no ratio");
    }

    #[test]
    fn an_account_takes_its_own_plan_then_its_readings_then_the_providers() {
        let f = PlansFile::bundled();
        let plan = |chosen, read, provider| f.account_plan(Provider::Anthropic, chosen, read, provider);
        assert_eq!(plan(Some("max20x"), Some("pro"), Some("pro")).as_deref(), Some("max20x"), "the user's choice wins");
        assert_eq!(plan(None, Some("pro"), Some("max5x")).as_deref(), Some("pro"), "a reading outranks the provider's plan");
        assert_eq!(plan(None, Some("max"), Some("max20x")).as_deref(), Some("max20x"));
        assert_eq!(plan(None, Some("max"), Some("pro")), None, "Max 5x or 20x is the user's to say");
        assert_eq!(plan(None, Some("max"), None), None);
        assert_eq!(plan(None, None, Some("pro")).as_deref(), Some("pro"));
        assert_eq!(plan(None, Some("team"), Some("pro")), None, "a reading no listed plan fits rules out the provider's plan too");
        assert_eq!(plan(Some("gone"), Some("pro"), None).as_deref(), Some("pro"), "a choice no longer listed is ignored");
        let codex = |read| f.account_plan(Provider::OpenAI, None, Some(read), Some("plus"));
        assert_eq!(codex("plus").as_deref(), Some("plus"));
        assert_eq!(codex("team").as_deref(), Some("business"), "Codex still calls Business \"team\"");
        assert_eq!(codex("edu").as_deref(), Some("enterprise"));
    }

    #[test]
    fn antigravity_plans_name_the_windows_agy_reports() {
        let f = PlansFile::bundled();
        let free = f.find(Provider::Google, "free").unwrap();
        assert_eq!(free.windows, ["seven_day"], "below Pro the quota refreshes weekly");
        let pro = f.above(Provider::Google, f.find(Provider::Google, "plus").unwrap()).unwrap();
        assert_eq!((pro.id.as_str(), pro.windows.as_slice()), ("pro", ["five_hour".to_owned(), "seven_day".to_owned()].as_slice()));
        assert!(f.find(Provider::Google, "api").unwrap().windows.is_empty());
    }
}
