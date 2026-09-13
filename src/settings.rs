use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::types::Seat;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlanPrices {
    pub t200: f64,
    pub t100: f64,
    pub t20: f64,
}

impl Default for PlanPrices {
    fn default() -> Self {
        Self {
            t200: 200.0,
            t100: 100.0,
            t20: 20.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BestInHouseMode {
    #[default]
    Absolute,
    PerPlan,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub start_at_login: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub subscriptions: BTreeMap<String, String>,
    pub subscription_counts: BTreeMap<String, usize>,
    pub best_in_house_mode: BestInHouseMode,
    pub escalation_minutes: f64,
    pub escalation_usd: f64,
    pub rho_override: Option<f64>,
    pub agent_hours: f64,
    pub orchestrators: usize,
    pub orchestrator_headroom_usd: Option<f64>,
    pub orchestrator_headroom_hours: Option<f64>,
    pub collaboration_root: Option<PathBuf>,
    pub draws: u32,
    pub assumption_span_pct: f64,
    pub competence_floors: BTreeMap<String, Option<f64>>,
    pub plan_prices: PlanPrices,
    pub vendor_overrides: BTreeMap<String, Option<f64>>,
    pub window_overrides: BTreeMap<String, Option<f64>>,
    pub cache_hours: u32,
    pub show_hallucination: bool,
    pub research_classes: BTreeMap<String, bool>,
    pub monotonic_class_competence: bool,
    pub allow_cross_harness_benchmark_proxies: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let mut vendor_overrides = BTreeMap::new();
        let known_vendors = [
            "openai",
            "anthropic",
            "google",
            "xai",
            "cursor",
            "cursor-api",
            "moonshot",
            "alibaba",
            "zai",
            "minimax",
            "opencode",
            "cognition",
            "muse",
            "deepseek",
        ];
        for vendor in known_vendors {
            vendor_overrides.insert(vendor.to_string(), None);
        }
        let competence_floors = Seat::ALL
            .into_iter()
            .map(|seat| (seat.name().to_string(), None))
            .collect();
        Self {
            start_at_login: false,
            start_minimized: false,
            close_to_tray: true,
            subscriptions: BTreeMap::new(),
            subscription_counts: BTreeMap::new(),
            best_in_house_mode: BestInHouseMode::Absolute,
            escalation_minutes: 60.0,
            escalation_usd: 0.0,
            rho_override: None,
            agent_hours: 40.0,
            orchestrators: 1,
            orchestrator_headroom_usd: None,
            orchestrator_headroom_hours: None,
            collaboration_root: None,
            draws: 4000,
            assumption_span_pct: 25.0,
            competence_floors,
            plan_prices: PlanPrices::default(),
            vendor_overrides,
            window_overrides: BTreeMap::new(),
            cache_hours: 1,
            show_hallucination: false,
            research_classes: crate::portfolio::WorkClass::ALL
                .into_iter()
                .map(|class| (class.name().to_owned(), false))
                .collect(),
            monotonic_class_competence: false,
            allow_cross_harness_benchmark_proxies: true,
        }
    }
}

impl Settings {
    pub fn scoring_matches(&self, other: &Self) -> bool {
        let mut scoring_settings = self.clone();
        scoring_settings.start_at_login = other.start_at_login;
        scoring_settings.start_minimized = other.start_minimized;
        scoring_settings.close_to_tray = other.close_to_tray;
        scoring_settings.best_in_house_mode = other.best_in_house_mode;
        scoring_settings.collaboration_root = other.collaboration_root.clone();
        scoring_settings == *other
    }

    pub fn normalize(mut self) -> Self {
        self.subscriptions.retain(|provider_id, plan_id| {
            crate::subscriptions::PROVIDERS.iter().any(|provider| {
                provider.id == provider_id && provider.plans.iter().any(|plan| plan.id == plan_id)
            })
        });
        self.subscription_counts.retain(|provider_id, count| {
            *count > 0
                && crate::subscriptions::PROVIDERS
                    .iter()
                    .any(|provider| provider.id == provider_id)
        });
        let defaults = Self::default();
        let finite = |value: f64, fallback: f64| {
            if value.is_finite() { value } else { fallback }
        };
        self.escalation_minutes =
            finite(self.escalation_minutes, defaults.escalation_minutes).clamp(0.0, 10080.0);
        self.escalation_usd =
            finite(self.escalation_usd, defaults.escalation_usd).clamp(0.0, 1_000_000.0);
        self.rho_override = self
            .rho_override
            .filter(|value| value.is_finite())
            .map(|value| value.clamp(0.0, 0.999));
        self.agent_hours = finite(self.agent_hours, defaults.agent_hours).clamp(1.0, 1680.0);
        self.orchestrators = self.orchestrators.clamp(1, 64);
        self.orchestrator_headroom_usd = self
            .orchestrator_headroom_usd
            .filter(|value| value.is_finite() && *value >= 0.0);
        self.orchestrator_headroom_hours = self
            .orchestrator_headroom_hours
            .filter(|value| value.is_finite() && *value >= 0.0);
        self.collaboration_root = self.collaboration_root.filter(|path| path.is_absolute());
        self.draws = self.draws.clamp(1, 20000);
        self.assumption_span_pct =
            finite(self.assumption_span_pct, defaults.assumption_span_pct).clamp(0.0, 90.0);
        for seat in Seat::ALL {
            let floor = self
                .competence_floors
                .entry(seat.name().to_string())
                .or_insert(None);
            if let Some(value) = floor {
                *floor = if value.is_finite() {
                    Some(value.clamp(0.0, 1.0))
                } else {
                    None
                };
            }
        }
        self.plan_prices.t200 =
            finite(self.plan_prices.t200, defaults.plan_prices.t200).clamp(1.0, 1000.0);
        self.plan_prices.t100 =
            finite(self.plan_prices.t100, defaults.plan_prices.t100).clamp(1.0, 1000.0);
        self.plan_prices.t20 =
            finite(self.plan_prices.t20, defaults.plan_prices.t20).clamp(1.0, 1000.0);
        self.cache_hours = self.cache_hours.clamp(1, 48);
        self.research_classes.retain(|class, _| {
            crate::portfolio::WorkClass::ALL
                .into_iter()
                .any(|known| known.name() == class)
        });
        for allowance in self
            .vendor_overrides
            .values_mut()
            .chain(self.window_overrides.values_mut())
        {
            if allowance.is_some_and(|value| !value.is_finite() || value < 0.0) {
                *allowance = None;
            }
        }
        self
    }

    pub fn subscription_count(&self, provider_id: &str) -> usize {
        self.subscription_counts
            .get(provider_id)
            .copied()
            .unwrap_or(1)
    }
}

pub fn settings_path() -> Result<PathBuf> {
    let dir = crate::update::install_config()?.data_dir;
    Ok(dir.join("settings.json"))
}

pub fn load_settings() -> Settings {
    let Ok(path) = settings_path() else {
        return Settings::default();
    };
    if !path.is_file() {
        return Settings::default();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&content) else {
                return Settings::default();
            };
            if value.get("agent_hours").is_none()
                && let Some(streams) = value.get("streams").and_then(serde_json::Value::as_f64)
            {
                value["agent_hours"] = serde_json::json!(streams * 56.0);
            }
            if let Ok(mut settings) = serde_json::from_value::<Settings>(value.clone()) {
                for vendor in Settings::default().vendor_overrides.into_keys() {
                    settings.vendor_overrides.entry(vendor).or_insert(None);
                }
                return settings.normalize();
            }
            let mut settings = Settings::default();
            if let Some(entries) = value
                .get("subscriptions")
                .and_then(serde_json::Value::as_object)
            {
                settings.subscriptions = entries
                    .iter()
                    .filter_map(|(provider, plan)| {
                        plan.as_str()
                            .map(|plan| (provider.clone(), plan.to_string()))
                    })
                    .collect();
            }
            if let Some(entries) = value
                .get("subscription_counts")
                .and_then(serde_json::Value::as_object)
            {
                settings.subscription_counts = entries
                    .iter()
                    .filter_map(|(provider, count)| {
                        usize::try_from(count.as_u64()?)
                            .ok()
                            .filter(|count| *count > 0)
                            .map(|count| (provider.clone(), count))
                    })
                    .collect();
            }
            if let Some(mode) = value.get("best_in_house_mode").cloned()
                && let Ok(mode) = serde_json::from_value(mode)
            {
                settings.best_in_house_mode = mode;
            }
            if let Some(v) = value.get("escalation_minutes").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.escalation_minutes = val;
            }
            if let Some(v) = value.get("escalation_usd").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.escalation_usd = val;
            }
            if let Some(v) = value.get("rho_override").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.rho_override = val;
            }
            if let Some(v) = value.get("agent_hours").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.agent_hours = val;
            }
            if let Some(v) = value.get("orchestrators").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.orchestrators = val;
            }
            if let Some(v) = value.get("collaboration_root").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.collaboration_root = val;
            }
            if let Some(v) = value.get("draws").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.draws = val;
            }
            if let Some(v) = value.get("assumption_span_pct").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.assumption_span_pct = val;
            }
            if let Some(v) = value.get("competence_floors").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.competence_floors = val;
            }
            if let Some(v) = value.get("plan_prices").cloned() {
                if let Ok(val) = serde_json::from_value(v.clone()) {
                    settings.plan_prices = val;
                } else if let Some(obj) = v.as_object() {
                    if let Some(t) = obj
                        .get("t200")
                        .and_then(|x| serde_json::from_value(x.clone()).ok())
                    {
                        settings.plan_prices.t200 = t;
                    }
                    if let Some(t) = obj
                        .get("t100")
                        .and_then(|x| serde_json::from_value(x.clone()).ok())
                    {
                        settings.plan_prices.t100 = t;
                    }
                    if let Some(t) = obj
                        .get("t20")
                        .and_then(|x| serde_json::from_value(x.clone()).ok())
                    {
                        settings.plan_prices.t20 = t;
                    }
                }
            }
            if let Some(v) = value.get("vendor_overrides").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.vendor_overrides = val;
            }
            if let Some(v) = value.get("window_overrides").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.window_overrides = val;
            }
            if let Some(v) = value.get("cache_hours").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.cache_hours = val;
            }
            if let Some(v) = value.get("show_hallucination").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.show_hallucination = val;
            }
            settings.normalize()
        }
        Err(_) => Settings::default(),
    }
}

pub fn save_settings(settings: &Settings) -> Result<()> {
    let path = settings_path()?;
    let content = serde_json::to_string_pretty(settings).context("serializing settings")?;
    std::fs::write(&path, content)
        .with_context(|| format!("writing settings to {}", path.display()))?;
    Ok(())
}
