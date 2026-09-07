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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub escalation_minutes: f64,
    pub escalation_usd: f64,
    pub rho: f64,
    pub agent_hours: f64,
    pub draws: u32,
    pub assumption_span_pct: f64,
    pub competence_floors: BTreeMap<String, Option<f64>>,
    pub plan_prices: PlanPrices,
    pub vendor_overrides: BTreeMap<String, Option<f64>>,
    pub cache_hours: u32,
    pub show_hallucination: bool,
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
            escalation_minutes: 60.0,
            escalation_usd: 0.0,
            rho: 0.653,
            agent_hours: 56.0,
            draws: 4000,
            assumption_span_pct: 25.0,
            competence_floors,
            plan_prices: PlanPrices::default(),
            vendor_overrides,
            cache_hours: 1,
            show_hallucination: false,
        }
    }
}

impl Settings {
    pub fn normalize(mut self) -> Self {
        let defaults = Self::default();
        let finite = |value: f64, fallback: f64| {
            if value.is_finite() { value } else { fallback }
        };
        self.escalation_minutes =
            finite(self.escalation_minutes, defaults.escalation_minutes).clamp(0.0, 10080.0);
        self.escalation_usd =
            finite(self.escalation_usd, defaults.escalation_usd).clamp(0.0, 1_000_000.0);
        self.rho = finite(self.rho, defaults.rho).clamp(0.0, 0.999);
        self.agent_hours = finite(self.agent_hours, defaults.agent_hours).clamp(1.0, 1680.0);
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
        for allowance in self.vendor_overrides.values_mut() {
            if allowance.is_some_and(|value| !value.is_finite() || value < 0.0) {
                *allowance = None;
            }
        }
        self
    }
}

pub fn settings_path() -> Result<PathBuf> {
    let dir = crate::update::data_dir()?;
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
            if let Some(v) = value.get("rho").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.rho = val;
            }
            if let Some(v) = value.get("agent_hours").cloned()
                && let Ok(val) = serde_json::from_value(v)
            {
                settings.agent_hours = val;
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
