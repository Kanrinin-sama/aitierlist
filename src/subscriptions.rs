use crate::types::Row;

pub struct Provider {
    pub id: &'static str,
    pub name: &'static str,
    pub plans: &'static [Plan],
}

pub struct Plan {
    pub id: &'static str,
    pub name: &'static str,
    pub monthly_price: f64,
    pub price_is_estimate: bool,
    pub monthly_allowance_low: f64,
    pub source_url: &'static str,
    pub allowance_basis: &'static str,
}

const ESTIMATE: &str = "Declared monthly API-equivalent usage estimate; not a published subscription quota. Weekly pool = estimate × 12/52.";

pub const PROVIDERS: &[Provider] = &[
    Provider {
        id: "openai",
        name: "Codex",
        plans: &[
            Plan {
                id: "codex-plus",
                name: "Plus",
                monthly_price: 20.0,
                price_is_estimate: false,
                monthly_allowance_low: 77.0,
                source_url: "https://developers.openai.com/codex/pricing",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "codex-pro-100",
                name: "Pro $100",
                monthly_price: 100.0,
                price_is_estimate: false,
                monthly_allowance_low: 384.0,
                source_url: "https://developers.openai.com/codex/pricing",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "codex-pro-200",
                name: "Pro $200",
                monthly_price: 200.0,
                price_is_estimate: false,
                monthly_allowance_low: 1538.0,
                source_url: "https://developers.openai.com/codex/pricing",
                allowance_basis: ESTIMATE,
            },
        ],
    },
    Provider {
        id: "anthropic",
        name: "Claude",
        plans: &[
            Plan {
                id: "claude-pro",
                name: "Pro",
                monthly_price: 20.0,
                price_is_estimate: false,
                monthly_allowance_low: 100.0,
                source_url: "https://support.claude.com/en/articles/11145838-using-claude-code-with-your-pro-or-max-plan",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "claude-max-5x",
                name: "Max 5×",
                monthly_price: 100.0,
                price_is_estimate: false,
                monthly_allowance_low: 500.0,
                source_url: "https://support.claude.com/en/articles/11145838-using-claude-code-with-your-pro-or-max-plan",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "claude-max-20x",
                name: "Max 20×",
                monthly_price: 200.0,
                price_is_estimate: false,
                monthly_allowance_low: 1100.0,
                source_url: "https://support.claude.com/en/articles/11145838-using-claude-code-with-your-pro-or-max-plan",
                allowance_basis: ESTIMATE,
            },
        ],
    },
    Provider {
        id: "muse",
        name: "Muse",
        plans: &[
            Plan {
                id: "muse-basic",
                name: "Everyday",
                monthly_price: 5.0,
                price_is_estimate: true,
                monthly_allowance_low: 5.0,
                source_url: "https://ai.developer.meta.com/docs/muse-code/subscriptions.md",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "muse-plus",
                name: "High Usage 3×",
                monthly_price: 15.0,
                price_is_estimate: true,
                monthly_allowance_low: 15.0,
                source_url: "https://ai.developer.meta.com/docs/muse-code/subscriptions.md",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "muse-pro",
                name: "Power Usage 10×",
                monthly_price: 50.0,
                price_is_estimate: true,
                monthly_allowance_low: 50.0,
                source_url: "https://ai.developer.meta.com/docs/muse-code/subscriptions.md",
                allowance_basis: ESTIMATE,
            },
        ],
    },
    Provider {
        id: "google",
        name: "Gemini",
        plans: &[
            Plan {
                id: "gemini-pro",
                name: "AI Pro",
                monthly_price: 19.99,
                price_is_estimate: false,
                monthly_allowance_low: 19.99,
                source_url: "https://one.google.com/about/google-ai-plans/",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "gemini-ultra-5x",
                name: "AI Ultra 5×",
                monthly_price: 99.99,
                price_is_estimate: false,
                monthly_allowance_low: 99.95,
                source_url: "https://one.google.com/about/google-ai-plans/",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "gemini-ultra-20x",
                name: "AI Ultra 20×",
                monthly_price: 199.99,
                price_is_estimate: false,
                monthly_allowance_low: 399.80,
                source_url: "https://one.google.com/about/google-ai-plans/",
                allowance_basis: ESTIMATE,
            },
        ],
    },
    Provider {
        id: "xai",
        name: "Grok",
        plans: &[
            Plan {
                id: "supergrok",
                name: "SuperGrok",
                monthly_price: 30.0,
                price_is_estimate: false,
                monthly_allowance_low: 19.0,
                source_url: "https://x.ai/pricing",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "supergrok-plus",
                name: "SuperGrok Plus",
                monthly_price: 100.0,
                price_is_estimate: false,
                monthly_allowance_low: 100.0,
                source_url: "https://x.ai/pricing",
                allowance_basis: ESTIMATE,
            },
            Plan {
                id: "supergrok-heavy",
                name: "SuperGrok Heavy",
                monthly_price: 300.0,
                price_is_estimate: true,
                monthly_allowance_low: 300.0,
                source_url: "https://x.ai/pricing",
                allowance_basis: "Monthly price and API-equivalent allowance are declared $300 estimates; the current price and numeric subscription quota are unconfirmed. Weekly pool = estimate × 12/52.",
            },
        ],
    },
];

pub fn eligible(provider: &Provider, plan: &Plan, row: &Row) -> bool {
    if row
        .effort
        .as_deref()
        .is_some_and(|effort| effort.eq_ignore_ascii_case("none"))
    {
        return false;
    }
    match provider.id {
        "openai" => {
            row.vendor == "openai"
                && row.harness == "Codex"
                && !row.model.to_lowercase().contains("spark")
        }
        "anthropic" => {
            row.vendor == "anthropic"
                && row.harness == "Claude Code"
                && (!is_fable(row) || plan.id != "claude-pro")
        }
        "muse" => {
            row.vendor == "muse"
                && (row.harness == "Muse Code" || row.harness.starts_with("Muse Code "))
        }
        "google" => {
            row.vendor == "google"
                && row.model.to_lowercase().contains("gemini")
                && (row.harness == "Gemini CLI" || row.harness.starts_with("Antigravity SDK "))
        }
        "xai" => {
            row.harness == "Grok Build"
                && (row.vendor == "xai" || row.model.starts_with("Composer 2.5"))
        }
        _ => false,
    }
}

pub fn is_fable(row: &Row) -> bool {
    row.model.to_lowercase().contains("fable")
}
