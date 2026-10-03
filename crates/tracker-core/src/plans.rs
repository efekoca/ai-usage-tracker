//! The plan list (`config/plans.json`): list prices, the order of each provider's personal
//! plans and the usage ratios the providers publish between them.

use crate::model::Provider;
use serde::Deserialize;
use std::collections::HashMap;

/// The plan file shipped with the app.
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

    /// The next personal plan up the ladder.
    pub fn above(&self, p: Provider, plan: &PlanDef) -> Option<&PlanDef> {
        let rung = plan.ladder?;
        self.plans(p).iter().filter(|x| x.ladder.is_some_and(|l| l > rung)).min_by_key(|x| x.ladder)
    }

    /// The next personal plan down the ladder.
    pub fn below(&self, p: Provider, plan: &PlanDef) -> Option<&PlanDef> {
        let rung = plan.ladder?;
        self.plans(p).iter().filter(|x| x.ladder.is_some_and(|l| l < rung)).max_by_key(|x| x.ladder)
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
}
