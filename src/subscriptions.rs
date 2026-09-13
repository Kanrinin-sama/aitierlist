use crate::settings::Settings;
use crate::types::{CapacityBasis, CapacityUnit, CapacityWindow, Row, WindowCap, WindowReset};

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
    pub windows: &'static [CapacityWindow<&'static str>],
    pub source_url: &'static str,
}

const OPENAI: &str = "https://developers.openai.com/codex/pricing";
const CLAUDE: &str =
    "https://support.claude.com/en/articles/11145838-using-claude-code-with-your-pro-or-max-plan";
const FABLE: &str =
    "https://support.claude.com/en/articles/15424964-claude-fable-models-on-your-plan";
const GEMINI: &str = "https://geminicli.com/docs/resources/quota-and-pricing/";
const MUSE: &str = "https://ai.developer.meta.com/docs/muse-code/subscriptions.md";
const GROK: &str = "https://docs.x.ai/grok/faq";

const fn weekly(
    price: f64,
    ratio: f64,
    source_url: &'static str,
    detail: &'static str,
) -> CapacityWindow<&'static str> {
    CapacityWindow {
        id: "weekly-usd",
        parent: None,
        unit: CapacityUnit::ApiEquivalentUsd,
        cap: WindowCap::Amount(price * ratio * 12.0 / 52.0),
        reset: WindowReset::Weekly,
        reset_at: None,
        basis: CapacityBasis::Anecdotal,
        reset_basis: CapacityBasis::Anecdotal,
        source_url,
        detail,
        reference_only: false,
    }
}

const CLAUDE_SHORT: CapacityWindow<&str> = CapacityWindow {
    id: "rolling-5h-usd",
    parent: Some("weekly-usd"),
    unit: CapacityUnit::ApiEquivalentUsd,
    cap: WindowCap::Unpublished,
    reset: WindowReset::Rolling { hours: 5 },
    reset_at: None,
    basis: CapacityBasis::Anecdotal,
    reset_basis: CapacityBasis::Published,
    source_url: CLAUDE,
    detail: "Five-hour USD amount unpublished; not enforced.",
    reference_only: false,
};

const FABLE_WEEKLY: CapacityWindow<&str> = CapacityWindow {
    id: "fable-weekly-usd",
    parent: Some("weekly-usd"),
    unit: CapacityUnit::ApiEquivalentUsd,
    cap: WindowCap::ParentFraction(0.5),
    reset: WindowReset::Weekly,
    reset_at: None,
    basis: CapacityBasis::Published,
    reset_basis: CapacityBasis::Published,
    source_url: FABLE,
    detail: "Fable on Max: published 50% ceiling; the derived USD amount inherits the parent's basis. Fable is excluded from Pro.",
    reference_only: false,
};

const fn native(
    id: &'static str,
    unit: CapacityUnit,
    cap: WindowCap,
    reset: WindowReset,
    source_url: &'static str,
    detail: &'static str,
) -> CapacityWindow<&'static str> {
    CapacityWindow {
        id,
        parent: None,
        unit,
        cap,
        reset,
        reset_at: None,
        basis: CapacityBasis::Published,
        reset_basis: CapacityBasis::Published,
        source_url,
        detail,
        reference_only: matches!(unit, CapacityUnit::Prompts)
            && matches!(cap, WindowCap::Range { .. }),
    }
}

const fn messages(
    id: &'static str,
    low: f64,
    high: f64,
    model: &'static str,
) -> CapacityWindow<&'static str> {
    CapacityWindow {
        reference_only: true,
        ..native(
            id,
            CapacityUnit::Messages,
            WindowCap::Range { low, high },
            WindowReset::Rolling { hours: 5 },
            OPENAI,
            model,
        )
    }
}

const fn grok_weekly(price: f64) -> CapacityWindow<&'static str> {
    CapacityWindow {
        reset_basis: CapacityBasis::Published,
        ..weekly(
            price,
            5.0,
            GROK,
            "Published shared weekly pool across Grok products; amount unpublished. Anecdotal USD = monthly price × 5 × 12/52; credits near face value.",
        )
    }
}

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
                source_url: OPENAI,
                windows: &[
                    weekly(
                        20.0,
                        16.0,
                        OPENAI,
                        "Anecdotal USD = monthly price × 16 × 12/52; vendor prior.",
                    ),
                    messages(
                        "gpt-6-astra-5h-messages",
                        5.0,
                        45.0,
                        "gpt-6-astra: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-sol-5h-messages",
                        10.0,
                        100.0,
                        "gpt-5-6-sol: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-terra-5h-messages",
                        25.0,
                        200.0,
                        "gpt-5-6-terra: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-luna-5h-messages",
                        250.0,
                        2000.0,
                        "gpt-5-6-luna: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-5-5h-messages",
                        15.0,
                        80.0,
                        "gpt-5-5: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-4-5h-messages",
                        20.0,
                        100.0,
                        "gpt-5-4: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-4-mini-5h-messages",
                        60.0,
                        350.0,
                        "gpt-5-4-mini: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                ],
            },
            Plan {
                id: "codex-pro-100",
                name: "Pro $100",
                monthly_price: 100.0,
                price_is_estimate: false,
                source_url: OPENAI,
                windows: &[
                    weekly(
                        100.0,
                        16.0,
                        OPENAI,
                        "Anecdotal USD = monthly price × 16 × 12/52; vendor prior.",
                    ),
                    messages(
                        "gpt-6-astra-5h-messages",
                        25.0,
                        225.0,
                        "gpt-6-astra: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-sol-5h-messages",
                        50.0,
                        500.0,
                        "gpt-5-6-sol: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-terra-5h-messages",
                        125.0,
                        1000.0,
                        "gpt-5-6-terra: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-luna-5h-messages",
                        1250.0,
                        10000.0,
                        "gpt-5-6-luna: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-5-5h-messages",
                        75.0,
                        400.0,
                        "gpt-5-5: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-4-5h-messages",
                        100.0,
                        500.0,
                        "gpt-5-4: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-4-mini-5h-messages",
                        300.0,
                        1750.0,
                        "gpt-5-4-mini: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                ],
            },
            Plan {
                id: "codex-pro-200",
                name: "Pro $200",
                monthly_price: 200.0,
                price_is_estimate: false,
                source_url: OPENAI,
                windows: &[
                    weekly(
                        200.0,
                        32.0,
                        OPENAI,
                        "Anecdotal USD = monthly price × 32 × 12/52; tier extrapolation.",
                    ),
                    messages(
                        "gpt-6-astra-5h-messages",
                        100.0,
                        900.0,
                        "gpt-6-astra: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-sol-5h-messages",
                        200.0,
                        2000.0,
                        "gpt-5-6-sol: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-terra-5h-messages",
                        500.0,
                        4000.0,
                        "gpt-5-6-terra: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-6-luna-5h-messages",
                        5000.0,
                        40000.0,
                        "gpt-5-6-luna: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-5-5h-messages",
                        300.0,
                        1600.0,
                        "gpt-5-5: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-4-5h-messages",
                        400.0,
                        2000.0,
                        "gpt-5-4: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                    messages(
                        "gpt-5-4-mini-5h-messages",
                        1200.0,
                        7000.0,
                        "gpt-5-4-mini: published workload-dependent local message estimate, not a fixed cap.",
                    ),
                ],
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
                source_url: CLAUDE,
                windows: &[
                    weekly(
                        20.0,
                        23.0,
                        CLAUDE,
                        "Anecdotal USD = monthly price × 23 × 12/52; field measurement on Max 20×.",
                    ),
                    CLAUDE_SHORT,
                ],
            },
            Plan {
                id: "claude-max-5x",
                name: "Max 5×",
                monthly_price: 100.0,
                price_is_estimate: false,
                source_url: CLAUDE,
                windows: &[
                    weekly(
                        100.0,
                        23.0,
                        CLAUDE,
                        "Anecdotal USD = monthly price × 23 × 12/52; field measurement on Max 20×.",
                    ),
                    CLAUDE_SHORT,
                    FABLE_WEEKLY,
                ],
            },
            Plan {
                id: "claude-max-20x",
                name: "Max 20×",
                monthly_price: 200.0,
                price_is_estimate: false,
                source_url: CLAUDE,
                windows: &[
                    weekly(
                        200.0,
                        23.0,
                        CLAUDE,
                        "Anecdotal USD = monthly price × 23 × 12/52; field measurement on Max 20×.",
                    ),
                    CLAUDE_SHORT,
                    FABLE_WEEKLY,
                ],
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
                source_url: MUSE,
                windows: &[
                    weekly(
                        5.0,
                        5.0,
                        MUSE,
                        "Anecdotal USD = monthly price × 5 × 12/52; credits near face value.",
                    ),
                    native(
                        "rolling-5h-prompts",
                        CapacityUnit::Prompts,
                        WindowCap::Range {
                            low: 10.0,
                            high: 50.0,
                        },
                        WindowReset::Rolling { hours: 5 },
                        MUSE,
                        "Published prompt range; workload-dependent, not a fixed prompt count. Monthly price is estimated.",
                    ),
                ],
            },
            Plan {
                id: "muse-plus",
                name: "High Usage 3×",
                monthly_price: 15.0,
                price_is_estimate: true,
                source_url: MUSE,
                windows: &[
                    weekly(
                        15.0,
                        5.0,
                        MUSE,
                        "Anecdotal USD = monthly price × 5 × 12/52; credits near face value.",
                    ),
                    native(
                        "rolling-5h-prompts",
                        CapacityUnit::Prompts,
                        WindowCap::Range {
                            low: 30.0,
                            high: 150.0,
                        },
                        WindowReset::Rolling { hours: 5 },
                        MUSE,
                        "Published prompt range; workload-dependent, not a fixed prompt count. Monthly price is estimated.",
                    ),
                ],
            },
            Plan {
                id: "muse-pro",
                name: "Power Usage 10×",
                monthly_price: 50.0,
                price_is_estimate: true,
                source_url: MUSE,
                windows: &[
                    weekly(
                        50.0,
                        5.0,
                        MUSE,
                        "Anecdotal USD = monthly price × 5 × 12/52; credits near face value.",
                    ),
                    native(
                        "rolling-5h-prompts",
                        CapacityUnit::Prompts,
                        WindowCap::Range {
                            low: 100.0,
                            high: 500.0,
                        },
                        WindowReset::Rolling { hours: 5 },
                        MUSE,
                        "Published prompt range; workload-dependent, not a fixed prompt count. Monthly price is estimated.",
                    ),
                ],
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
                source_url: GEMINI,
                windows: &[
                    weekly(
                        19.99,
                        5.0,
                        GEMINI,
                        "Anecdotal USD = monthly price × 5 × 12/52; credits near face value.",
                    ),
                    native(
                        "gemini-cli-daily-requests",
                        CapacityUnit::Requests,
                        WindowCap::Amount(1500.0),
                        WindowReset::Daily,
                        GEMINI,
                        "Gemini CLI requests per user per day; Ultra tier names do not multiply this quota.",
                    ),
                ],
            },
            Plan {
                id: "gemini-ultra-5x",
                name: "AI Ultra 5×",
                monthly_price: 99.99,
                price_is_estimate: false,
                source_url: GEMINI,
                windows: &[
                    weekly(
                        99.99,
                        5.0,
                        GEMINI,
                        "Anecdotal USD = monthly price × 5 × 12/52; credits near face value.",
                    ),
                    native(
                        "gemini-cli-daily-requests",
                        CapacityUnit::Requests,
                        WindowCap::Amount(2000.0),
                        WindowReset::Daily,
                        GEMINI,
                        "Gemini CLI requests per user per day; Ultra tier names do not multiply this quota.",
                    ),
                ],
            },
            Plan {
                id: "gemini-ultra-20x",
                name: "AI Ultra 20×",
                monthly_price: 199.99,
                price_is_estimate: false,
                source_url: GEMINI,
                windows: &[
                    weekly(
                        199.99,
                        5.0,
                        GEMINI,
                        "Anecdotal USD = monthly price × 5 × 12/52; credits near face value.",
                    ),
                    native(
                        "gemini-cli-daily-requests",
                        CapacityUnit::Requests,
                        WindowCap::Amount(2000.0),
                        WindowReset::Daily,
                        GEMINI,
                        "Gemini CLI requests per user per day; Ultra tier names do not multiply this quota.",
                    ),
                ],
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
                source_url: GROK,
                windows: &[grok_weekly(30.0)],
            },
            Plan {
                id: "supergrok-plus",
                name: "SuperGrok Plus",
                monthly_price: 100.0,
                price_is_estimate: false,
                source_url: GROK,
                windows: &[grok_weekly(100.0)],
            },
            Plan {
                id: "supergrok-heavy",
                name: "SuperGrok Heavy",
                monthly_price: 300.0,
                price_is_estimate: true,
                source_url: GROK,
                windows: &[grok_weekly(300.0)],
            },
        ],
    },
];

const fn prior(low: f64, high: f64) -> CapacityWindow<&'static str> {
    CapacityWindow {
        basis: CapacityBasis::TransferredPrior,
        reset_basis: CapacityBasis::TransferredPrior,
        cap: WindowCap::Range {
            low: low * 12.0 / 52.0,
            high: high * 12.0 / 52.0,
        },
        ..weekly(
            0.0,
            0.0,
            "",
            "Transferred anecdotal ranking prior; unverified monthly USD endpoints × 12/52.",
        )
    }
}

pub(crate) const RANKING_PRIORS: &[Provider] = &[
    Provider {
        id: "cursor",
        name: "cursor",
        plans: &[
            Plan {
                id: "cursor-200",
                name: "$200",
                monthly_price: 200.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(228.0, 228.0)],
            },
            Plan {
                id: "cursor-60",
                name: "$60",
                monthly_price: 60.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(34.0, 92.0)],
            },
            Plan {
                id: "cursor-20",
                name: "$20",
                monthly_price: 20.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(11.0, 11.0)],
            },
        ],
    },
    Provider {
        id: "cursor-api",
        name: "cursor-api",
        plans: &[
            Plan {
                id: "cursor-api-200",
                name: "$200",
                monthly_price: 200.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(92.31, 92.31)],
            },
            Plan {
                id: "cursor-api-60",
                name: "$60",
                monthly_price: 60.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(16.15, 16.15)],
            },
            Plan {
                id: "cursor-api-20",
                name: "$20",
                monthly_price: 20.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(4.62, 4.62)],
            },
        ],
    },
    Provider {
        id: "moonshot",
        name: "moonshot",
        plans: &[
            Plan {
                id: "moonshot-199",
                name: "$199",
                monthly_price: 199.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(200.0, 200.0)],
            },
            Plan {
                id: "moonshot-99",
                name: "$99",
                monthly_price: 99.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(100.0, 100.0)],
            },
            Plan {
                id: "moonshot-39",
                name: "$39",
                monthly_price: 39.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(33.0, 33.0)],
            },
            Plan {
                id: "moonshot-19",
                name: "$19",
                monthly_price: 19.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(6.7, 6.7)],
            },
        ],
    },
    Provider {
        id: "alibaba",
        name: "alibaba",
        plans: &[Plan {
            id: "alibaba-50",
            name: "$50",
            monthly_price: 50.0,
            price_is_estimate: true,
            source_url: "",
            windows: &[CapacityWindow {
                id: "monthly-requests",
                unit: CapacityUnit::Requests,
                cap: WindowCap::Amount(90000.0),
                reset: WindowReset::Monthly,
                reset_at: None,
                detail: "Transferred anecdotal native request prior; stored only, never converted to USD.",
                ..prior(0.0, 0.0)
            }],
        }],
    },
    Provider {
        id: "zai",
        name: "zai",
        plans: &[
            Plan {
                id: "zai-168",
                name: "$168",
                monthly_price: 168.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(428.0, 568.0)],
            },
            Plan {
                id: "zai-80",
                name: "$80",
                monthly_price: 80.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(184.0, 243.0)],
            },
            Plan {
                id: "zai-18",
                name: "$18",
                monthly_price: 18.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(31.0, 41.0)],
            },
        ],
    },
    Provider {
        id: "minimax",
        name: "minimax",
        plans: &[
            Plan {
                id: "minimax-132",
                name: "$132",
                monthly_price: 132.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(132.0, 132.0)],
            },
            Plan {
                id: "minimax-55",
                name: "$55",
                monthly_price: 55.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(55.0, 55.0)],
            },
            Plan {
                id: "minimax-22",
                name: "$22",
                monthly_price: 22.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(22.0, 22.0)],
            },
        ],
    },
    Provider {
        id: "opencode",
        name: "opencode",
        plans: &[Plan {
            id: "opencode-10",
            name: "$10",
            monthly_price: 10.0,
            price_is_estimate: true,
            source_url: "",
            windows: &[prior(60.0, 60.0)],
        }],
    },
    Provider {
        id: "cognition",
        name: "cognition",
        plans: &[
            Plan {
                id: "cognition-200",
                name: "$200",
                monthly_price: 200.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(200.0, 200.0)],
            },
            Plan {
                id: "cognition-20",
                name: "$20",
                monthly_price: 20.0,
                price_is_estimate: true,
                source_url: "",
                windows: &[prior(20.0, 20.0)],
            },
        ],
    },
];

pub fn plan_for(provider_id: &str, budget: f64) -> Option<&'static Plan> {
    PROVIDERS
        .iter()
        .chain(RANKING_PRIORS)
        .find(|provider| provider.id == provider_id)?
        .plans
        .iter()
        .filter(|plan| plan.monthly_price <= budget)
        .max_by(|left, right| left.monthly_price.total_cmp(&right.monthly_price))
}

pub fn window_override_key(provider_id: &str, plan_id: &str, window_id: &str) -> String {
    format!("{provider_id}/{plan_id}/{window_id}")
}

impl Plan {
    pub fn rate_infeasibility(
        &self,
        provider_id: &str,
        settings: &Settings,
        demand: crate::types::CapacityDemand,
        duration_hours: f64,
        fable: bool,
    ) -> Option<String> {
        let windows = self.resolved_windows(provider_id, settings);
        windows.iter().find_map(|window| {
            if !window.applies_to(&windows, fable) {
                return None;
            }
            let ceiling = window.capacity_limit(&windows).ok()?;
            let amount = demand.amount(window.unit)?;
            let committed = if window.reset.is_budget() {
                settings.orchestrators as f64
                    * amount
                    * window.reset.commitment_hours(settings.agent_hours)
                    / settings.agent_hours
            } else {
                settings.orchestrators as f64 * amount / duration_hours
                    * window.reset.commitment_hours(settings.agent_hours)
            };
            (committed > ceiling).then(|| {
                format!(
                    "capacity-infeasible: {} commits {committed:.4} {} > {ceiling:.4} {}",
                    window.id,
                    window.unit.label(),
                    window.unit.label()
                )
            })
        })
    }

    pub fn weekly_window(&self) -> Option<&CapacityWindow<&'static str>> {
        self.windows.iter().find(|window| {
            window.parent.is_none()
                && window.unit == CapacityUnit::ApiEquivalentUsd
                && window.reset == WindowReset::Weekly
                && !window.reference_only
        })
    }

    pub fn override_amount(
        &self,
        provider_id: &str,
        window: &CapacityWindow<&str>,
        settings: &Settings,
    ) -> Option<f64> {
        if window.basis == CapacityBasis::Published
            && matches!(window.cap, WindowCap::ParentFraction(_))
        {
            return None;
        }
        settings
            .window_overrides
            .get(&window_override_key(provider_id, self.id, window.id))
            .copied()
            .flatten()
            .or_else(|| {
                self.weekly_window()
                    .filter(|parent| parent.id == window.id)
                    .and_then(|_| {
                        settings
                            .vendor_overrides
                            .get(provider_id)
                            .copied()
                            .flatten()
                    })
            })
            .filter(|amount| amount.is_finite() && *amount >= 0.0)
    }

    pub fn weekly_bounds(&self, provider_id: &str, settings: &Settings) -> Option<(f64, f64)> {
        let window = self.weekly_window()?;
        self.override_amount(provider_id, window, settings)
            .map(|amount| (amount, amount))
            .or_else(|| window.cap.bounds(None))
    }

    pub fn fable_capacity(&self, provider_id: &str, settings: &Settings) -> Option<f64> {
        let windows = self.resolved_windows(provider_id, settings);
        windows
            .iter()
            .find(|window| window.id == FABLE_WEEKLY.id)
            .and_then(|window| window.amount_bounds(&windows))
            .map(|(low, _)| low)
    }
    pub fn resolved_windows(&self, provider_id: &str, settings: &Settings) -> Vec<CapacityWindow> {
        let windows: Vec<_> = self
            .windows
            .iter()
            .map(|window| {
                let override_amount = self.override_amount(provider_id, window, settings);
                CapacityWindow {
                    id: window.id.to_owned(),
                    parent: window.parent.map(str::to_owned),
                    unit: window.unit,
                    cap: override_amount.map(WindowCap::Amount).unwrap_or(window.cap),
                    reset: window.reset,
                    reset_at: window.reset_at.map(str::to_owned),
                    basis: if override_amount.is_some() {
                        CapacityBasis::UserOverride
                    } else {
                        window.basis
                    },
                    reset_basis: window.reset_basis,
                    source_url: window.source_url.to_owned(),
                    detail: if override_amount.is_some() {
                        format!(
                            "User amount; table default: {:?} ({}). {}",
                            window.cap,
                            window.basis.label(),
                            window.detail
                        )
                    } else {
                        window.detail.to_owned()
                    },
                    reference_only: window.reference_only,
                }
            })
            .collect();
        windows
            .iter()
            .map(|window| {
                let mut resolved = window.clone();
                if matches!(window.cap, WindowCap::ParentFraction(_)) {
                    resolved.cap = match window.amount_bounds(&windows) {
                        Some((low, high)) if low == high => WindowCap::Amount(low),
                        Some((low, high)) => WindowCap::Range { low, high },
                        None => WindowCap::Unpublished,
                    };
                    resolved.detail = window.summary(&windows, settings.agent_hours);
                }
                resolved
            })
            .collect()
    }

    pub fn capacity_summary(&self, provider_id: &str, settings: &Settings) -> String {
        let windows = self.resolved_windows(provider_id, settings);
        windows
            .iter()
            .map(|window| {
                format!(
                    "{} Source: {}",
                    window.summary(&windows, settings.agent_hours),
                    window.source_url
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn weekly_basis(&self, provider_id: &str, settings: &Settings) -> &'static str {
        self.weekly_window()
            .map(|window| {
                if self
                    .override_amount(provider_id, window, settings)
                    .is_some()
                {
                    CapacityBasis::UserOverride.label()
                } else {
                    window.basis.label()
                }
            })
            .unwrap_or("no weekly USD window")
    }
}

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
        _ => provider.id == row.vendor && row.harness != "model",
    }
}

pub fn is_fable(row: &Row) -> bool {
    row.model.to_lowercase().contains("fable")
}
