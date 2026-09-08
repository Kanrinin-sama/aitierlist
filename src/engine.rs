use crate::settings::Settings;
use crate::types::{
    Benchmark, CacheState, CandidatePick, ExcludedCandidate, FrontierPoint, Pick, Row,
    ScenarioPick, Seat, SeatTierFrontier, SeatTierPick, Table, Tier,
};
use std::collections::BTreeSet;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const VENDORS: [&str; 14] = [
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
const CODING: [(Benchmark, f64); 2] = [
    (Benchmark::Swe, 113.0 / 202.0),
    (Benchmark::Terminal, 89.0 / 202.0),
];
const DEBUGGER: [(Benchmark, f64); 4] = [
    (Benchmark::Swe, 113.0 / 404.0),
    (Benchmark::Terminal, 89.0 / 404.0),
    (Benchmark::Gpqa, 0.25),
    (Benchmark::Hle, 0.25),
];
const REVIEWER: [(Benchmark, f64); 3] = [
    (Benchmark::Qna, 0.5),
    (Benchmark::Gpqa, 0.25),
    (Benchmark::Hle, 0.25),
];
const SANITY: [(Benchmark, f64); 1] = [(Benchmark::Qna, 1.0)];
const COMPREHENSION: [(Benchmark, f64); 2] = [(Benchmark::Lcr, 0.5), (Benchmark::Qna, 0.5)];
const QNA: [(Benchmark, f64); 1] = [(Benchmark::Qna, 1.0)];
const TERMINAL: [(Benchmark, f64); 1] = [(Benchmark::Terminal, 1.0)];

fn pass(row: &Row, benchmark: Benchmark) -> Option<f64> {
    let value = match benchmark {
        Benchmark::Swe => row.swe,
        Benchmark::Terminal => row.term,
        Benchmark::Qna => row.qna,
        Benchmark::Gpqa => row.gpqa,
        Benchmark::Hle => row.hle,
        Benchmark::Lcr => row.lcr,
    }?;
    (value.is_finite() && (0.0..=1.0).contains(&value))
        .then(|| row.retry.adjusted_pass(benchmark, value))
}

pub fn competence(row: &Row, seat: Seat) -> Option<f64> {
    let value = competence_components(row, seat)?
        .iter()
        .map(|(_, score, weight)| score * weight)
        .sum::<f64>();
    (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(value)
}

pub fn competence_components(row: &Row, seat: Seat) -> Option<Vec<(&'static str, f64, f64)>> {
    if seat == Seat::Orchestrator {
        let smart = row.smart?;
        let gpqa = pass(row, Benchmark::Gpqa)?;
        return (smart.is_finite() && (0.0..=1.0).contains(&smart)).then_some(vec![
            ("GPQA reasoning", gpqa, 0.5),
            ("Intelligence index", smart, 0.5),
        ]);
    }
    let profile = match seat {
        Seat::Implementer => &CODING[..],
        Seat::Debugger => &DEBUGGER[..],
        Seat::Reviewer => &REVIEWER[..],
        Seat::Sanity => &SANITY[..],
        Seat::Comprehension => &COMPREHENSION[..],
        Seat::Orchestrator => unreachable!(),
    };
    profile
        .iter()
        .map(|(benchmark, weight)| Some((benchmark.name(), pass(row, *benchmark)?, *weight)))
        .collect()
}

pub fn reference_workload(seat: Seat) -> &'static [(Benchmark, f64)] {
    match seat {
        Seat::Implementer | Seat::Debugger => &CODING,
        _ => &QNA,
    }
}

pub fn reference_workload_name(seat: Seat) -> &'static str {
    match seat {
        Seat::Implementer | Seat::Debugger => "coding reference workload",
        _ => "repository Q&A reference workload",
    }
}

#[derive(Clone, Copy)]
pub enum Allowance {
    Usd(f64, f64),
    Calls(f64),
}

fn priced_plan_for(vendor: &str, budget: f64) -> Option<(f64, Allowance)> {
    use Allowance::{Calls, Usd};
    let plans: &[(f64, Allowance)] = match vendor {
        "openai" => &[
            (200.0, Usd(1538.0, 2272.0)),
            (100.0, Usd(384.0, 568.0)),
            (20.0, Usd(77.0, 114.0)),
        ],
        "anthropic" => &[
            (200.0, Usd(1100.0, 1300.0)),
            (100.0, Usd(500.0, 650.0)),
            (20.0, Usd(100.0, 130.0)),
        ],
        "google" => &[
            (199.99, Usd(19.99 * 20.0, 19.99 * 20.0)),
            (99.99, Usd(19.99 * 5.0, 19.99 * 5.0)),
            (19.99, Usd(19.99, 19.99)),
        ],
        "muse" => &[
            (50.0, Usd(50.0, 50.0)),
            (15.0, Usd(15.0, 15.0)),
            (5.0, Usd(5.0, 5.0)),
        ],
        "xai" => &[
            (300.0, Usd(300.0, 300.0)),
            (100.0, Usd(100.0, 100.0)),
            (30.0, Usd(19.0, 115.0)),
        ],
        "cursor" => &[
            (200.0, Usd(228.0, 228.0)),
            (60.0, Usd(34.0, 92.0)),
            (20.0, Usd(11.0, 11.0)),
        ],
        "cursor-api" => &[
            (200.0, Usd(92.31, 92.31)),
            (60.0, Usd(16.15, 16.15)),
            (20.0, Usd(4.62, 4.62)),
        ],
        "moonshot" => &[
            (199.0, Usd(200.0, 200.0)),
            (99.0, Usd(100.0, 100.0)),
            (39.0, Usd(33.0, 33.0)),
            (19.0, Usd(6.7, 6.7)),
        ],
        "alibaba" => &[(50.0, Calls(90000.0))],
        "zai" => &[
            (168.0, Usd(428.0, 568.0)),
            (80.0, Usd(184.0, 243.0)),
            (18.0, Usd(31.0, 41.0)),
        ],
        "minimax" => &[
            (132.0, Usd(132.0, 132.0)),
            (55.0, Usd(55.0, 55.0)),
            (22.0, Usd(22.0, 22.0)),
        ],
        "opencode" => &[(10.0, Usd(60.0, 60.0))],
        "cognition" => &[(200.0, Usd(200.0, 200.0)), (20.0, Usd(20.0, 20.0))],
        _ => &[],
    };
    plans
        .iter()
        .find(|(price, _)| *price <= budget)
        .map(|(price, plan)| (*price, *plan))
}

pub fn plan_for(vendor: &str, budget: f64) -> Option<Allowance> {
    priced_plan_for(vendor, budget).map(|(_, allowance)| allowance)
}

pub fn subscription_price(row: &Row, tier: Tier, settings: &Settings) -> Option<f64> {
    if tier == Tier::Api
        || settings
            .vendor_overrides
            .get(&row.vendor)
            .copied()
            .flatten()
            .is_some()
    {
        return None;
    }
    let budget = match tier {
        Tier::Api => unreachable!(),
        Tier::T200 => settings.plan_prices.t200,
        Tier::T100 => settings.plan_prices.t100,
        Tier::T20 => settings.plan_prices.t20,
    };
    priced_plan_for(&row.vendor, budget).map(|(price, _)| price)
}

fn plan_eligible(row: &Row, plan: Option<Allowance>) -> bool {
    let Some(plan) = plan else { return false };
    if matches!(plan, Allowance::Calls(_))
        && !row
            .usd_per_step
            .is_some_and(|value| value.is_finite() && value > 0.0)
    {
        return false;
    }
    !row.harness.eq_ignore_ascii_case("Claude Code")
        || !["GLM-5.1", "GLM-5.2", "Qwen3.8 Max"].iter().any(|model| {
            crate::aa::family_key("", &row.model_key)
                == crate::aa::family_key("", &crate::aa::harness_key(model))
        })
}

#[derive(Clone, Copy, Default)]
pub struct Cycle {
    pub wall: f64,
    pub spend: f64,
    pub model_spend: f64,
    pub escalation_spend: f64,
    pub unfinished: f64,
    pub escalation_seconds: f64,
    pub agent_seconds: f64,
}

#[derive(Clone, Copy)]
pub struct ResourceAdjustment {
    pub row_index: usize,
    pub runtime_multiplier: f64,
    pub model_cost_multiplier: f64,
    pub allowance_multiplier: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TimeBasis {
    Token,
    Pooled,
}

#[derive(Clone)]
struct Assumptions {
    name: String,
    runtime: f64,
    model_cost: f64,
    rho_multiplier: f64,
    escalation_time: f64,
    allowance_high: Option<bool>,
    time_basis: TimeBasis,
}

fn baseline(time_basis: TimeBasis) -> Assumptions {
    Assumptions {
        name: format!(
            "Baseline · {} time",
            if time_basis == TimeBasis::Token {
                "token-proportional"
            } else {
                "pooled"
            }
        ),
        runtime: 1.0,
        model_cost: 1.0,
        rho_multiplier: 1.0,
        escalation_time: 1.0,
        allowance_high: None,
        time_basis,
    }
}

fn scenarios(settings: &Settings) -> Vec<Assumptions> {
    let multipliers = [
        1.0 - settings.assumption_span_pct / 100.0,
        1.0 + settings.assumption_span_pct / 100.0,
    ];
    let mut scenarios = Vec::with_capacity(66);
    for time_basis in [TimeBasis::Token, TimeBasis::Pooled] {
        scenarios.push(baseline(time_basis));
        for mask in 0..32_u8 {
            scenarios.push(Assumptions {
                name: format!("{} time · runtime {} · cost {} · correlation {} · escalation {} · allowance {}", if time_basis == TimeBasis::Token { "Token" } else { "Pooled" }, if mask & 1 == 0 { "low" } else { "high" }, if mask & 2 == 0 { "low" } else { "high" }, if mask & 4 == 0 { "low" } else { "high" }, if mask & 8 == 0 { "low" } else { "high" }, if mask & 16 == 0 { "low" } else { "high" }),
                runtime: multipliers[usize::from(mask & 1 != 0)],
                model_cost: multipliers[usize::from(mask & 2 != 0)],
                rho_multiplier: multipliers[usize::from(mask & 4 != 0)],
                escalation_time: multipliers[usize::from(mask & 8 != 0)],
                allowance_high: Some(mask & 16 != 0),
                time_basis,
            });
        }
    }
    scenarios
}

fn cycles_for(
    row: &Row,
    mix: &[(Benchmark, f64)],
    settings: &Settings,
    assumptions: &Assumptions,
    runtime_multiplier: f64,
    model_cost_multiplier: f64,
) -> Option<[Cycle; 64]> {
    let mut totals = [Cycle::default(); 64];
    for &(benchmark, weight) in mix {
        let metric = row
            .task_metrics
            .iter()
            .find(|metric| metric.benchmark == benchmark)?;
        let seconds = if assumptions.time_basis == TimeBasis::Token {
            metric.seconds
        } else {
            metric.pooled_seconds
        };
        if !metric.pass.is_finite()
            || !(0.0..=1.0).contains(&metric.pass)
            || !seconds.is_finite()
            || seconds <= 0.0
            || !metric.usd.is_finite()
            || metric.usd < 0.0
        {
            return None;
        }
        let resolved = match benchmark {
            Benchmark::Swe => row.retry.deepswe,
            Benchmark::Terminal => row.retry.terminal_bench,
            Benchmark::Qna => row.retry.qna,
            _ => return None,
        };
        let rho = (settings.rho_override.unwrap_or(resolved.rho) * assumptions.rho_multiplier)
            .clamp(0.0, 1.0 - f64::EPSILON);
        let ratio = rho / (1.0 - rho);
        let pass = row.retry.adjusted_pass(benchmark, metric.pass);
        let mut reached = 1.0;
        let mut attempts = 0.0;
        for (attempt, total) in totals.iter_mut().enumerate() {
            attempts += reached;
            reached *= 1.0 - pass / (1.0 + attempt as f64 * ratio);
            let agent_seconds = attempts * seconds * assumptions.runtime * runtime_multiplier;
            let escalation_seconds =
                reached * settings.escalation_minutes * 60.0 * assumptions.escalation_time;
            total.agent_seconds += weight * agent_seconds;
            total.escalation_seconds += weight * escalation_seconds;
            total.wall += weight * (agent_seconds + escalation_seconds);
            total.model_spend +=
                weight * attempts * metric.usd * assumptions.model_cost * model_cost_multiplier;
            total.escalation_spend += weight * reached * settings.escalation_usd;
            total.unfinished += weight * reached;
        }
    }
    for total in &mut totals {
        total.spend = total.model_spend + total.escalation_spend;
    }
    Some(totals)
}

pub fn implementation_cycle(row: &Row, settings: &Settings) -> Option<Cycle> {
    let mix = if row.harness == "model" {
        &TERMINAL[..]
    } else {
        &CODING[..]
    };
    cycles_for(row, mix, settings, &baseline(TimeBasis::Token), 1.0, 1.0)?
        .into_iter()
        .reduce(|best, next| {
            if (1.0 - next.unfinished) / next.wall > (1.0 - best.unfinished) / best.wall {
                next
            } else {
                best
            }
        })
}

fn allowance(row: &Row, plan: Option<Allowance>, override_amount: Option<f64>) -> (f64, f64) {
    if let Some(amount) = override_amount {
        return (amount, amount);
    }
    let conversion = if matches!(plan, Some(Allowance::Calls(_))) {
        row.usd_per_step.unwrap_or(0.0)
    } else {
        1.0
    };
    match plan {
        Some(Allowance::Usd(low, high)) => (
            low * 12.0 / 52.0 * conversion,
            high * 12.0 / 52.0 * conversion,
        ),
        Some(Allowance::Calls(amount)) => {
            let amount = amount * 12.0 / 52.0 * conversion;
            (amount, amount)
        }
        None => (0.0, 0.0),
    }
}

pub fn has_positive_allowance(row: &Row, tier: Tier, settings: &Settings) -> bool {
    if tier == Tier::Api {
        return false;
    }
    let budget = match tier {
        Tier::Api => unreachable!(),
        Tier::T200 => settings.plan_prices.t200,
        Tier::T100 => settings.plan_prices.t100,
        Tier::T20 => settings.plan_prices.t20,
    };
    let plan = plan_for(&row.vendor, budget);
    let override_amount = settings
        .vendor_overrides
        .get(&row.vendor)
        .copied()
        .flatten();
    let range = allowance(row, plan, override_amount);
    range.1 > 0.0
}

fn fixed_tasks(time: f64, cost: f64, amount: f64) -> f64 {
    if cost == 0.0 {
        time
    } else {
        time.min(amount / cost)
    }
}

fn tasks(
    cycle: Cycle,
    tier: Tier,
    settings: &Settings,
    range: (f64, f64),
    point: Option<bool>,
) -> f64 {
    let time = settings.agent_hours * 3600.0 / cycle.wall;
    let cycles = if tier == Tier::Api {
        time
    } else {
        let amount = match point {
            Some(false) => range.0,
            Some(true) => range.1,
            None => (range.0 + range.1) / 2.0,
        };
        fixed_tasks(time, cycle.model_spend, amount)
    };
    cycles * (1.0 - cycle.unfinished)
}

fn cycle_capacity(cycle: Cycle, tier: Tier, settings: &Settings, range: (f64, f64)) -> f64 {
    let time = settings.agent_hours * 3600.0 / cycle.wall;
    if tier == Tier::Api {
        return time;
    }
    let amount = (range.0 + range.1) / 2.0;
    fixed_tasks(time, cycle.model_spend, amount)
}

struct Policy {
    row_index: usize,
    competence: Option<f64>,
    range: (f64, f64),
    calls: bool,
    limit: usize,
    cycle: Cycle,
    nominal: f64,
}

struct Evaluation {
    selected: usize,
    capacity: Vec<f64>,
    best_capacity: Vec<f64>,
}

fn policy_order(
    left: usize,
    right: usize,
    policies: &[Policy],
    capacity: &[f64],
    rows: &[Row],
) -> std::cmp::Ordering {
    policies[right]
        .nominal
        .total_cmp(&policies[left].nominal)
        .then_with(|| capacity[left].total_cmp(&capacity[right]))
        .then_with(|| {
            policies[right]
                .competence
                .partial_cmp(&policies[left].competence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| policies[left].limit.cmp(&policies[right].limit))
        .then_with(|| {
            rows[policies[left].row_index]
                .display_name()
                .cmp(&rows[policies[right].row_index].display_name())
        })
        .then_with(|| {
            rows[policies[left].row_index]
                .harness
                .cmp(&rows[policies[right].row_index].harness)
        })
        .then_with(|| {
            rows[policies[left].row_index]
                .effort
                .cmp(&rows[policies[right].row_index].effort)
        })
}

fn analyze(
    rows: &[Row],
    seat: Seat,
    tier: Tier,
    settings: &Settings,
    adjustment: Option<ResourceAdjustment>,
    include_frontier: bool,
) -> (Option<Pick>, SeatTierFrontier) {
    let budget = match tier {
        Tier::Api => -1.0,
        Tier::T200 => settings.plan_prices.t200,
        Tier::T100 => settings.plan_prices.t100,
        Tier::T20 => settings.plan_prices.t20,
    };
    let plans = VENDORS.map(|vendor| plan_for(vendor, budget));
    let nominal_assumptions = baseline(TimeBasis::Token);
    let mut policies = Vec::new();
    let mut excluded = Vec::new();
    for (row_index, row) in rows.iter().enumerate() {
        if row.harness == "model" {
            excluded.push(ExcludedCandidate {
                row_index,
                reason: "inventory-only model".to_string(),
            });
            continue;
        }
        let competence = competence(row, seat);
        if matches!(seat, Seat::Implementer | Seat::Debugger)
            && !self::competence(row, Seat::Implementer).is_some_and(|value| value > 0.0)
        {
            excluded.push(ExcludedCandidate {
                row_index,
                reason: "missing or zero coding pass".to_string(),
            });
            continue;
        }
        let vendor = VENDORS.iter().position(|vendor| *vendor == row.vendor);
        let plan = vendor.and_then(|index| plans[index]);
        let override_amount = settings
            .vendor_overrides
            .get(&row.vendor)
            .copied()
            .flatten();
        if tier != Tier::Api
            && !plan_eligible(row, plan)
            && !(plan.is_none() && override_amount.is_some())
        {
            excluded.push(ExcludedCandidate {
                row_index,
                reason: "not available in this tier".to_string(),
            });
            continue;
        }
        let row_adjustment = adjustment.filter(|value| value.row_index == row_index);
        let runtime_multiplier = row_adjustment
            .map(|value| value.runtime_multiplier)
            .unwrap_or(1.0);
        let model_cost_multiplier = row_adjustment
            .map(|value| value.model_cost_multiplier)
            .unwrap_or(1.0);
        let allowance_multiplier = row_adjustment
            .map(|value| value.allowance_multiplier)
            .unwrap_or(1.0);
        let mut range = allowance(row, plan, override_amount);
        let calls = override_amount.is_none() && matches!(plan, Some(Allowance::Calls(_)));
        let range_multiplier =
            allowance_multiplier * if calls { model_cost_multiplier } else { 1.0 };
        range.0 *= range_multiplier;
        range.1 *= range_multiplier;
        let Some(cycles) = cycles_for(
            row,
            reference_workload(seat),
            settings,
            &nominal_assumptions,
            runtime_multiplier,
            model_cost_multiplier,
        ) else {
            excluded.push(ExcludedCandidate {
                row_index,
                reason: "missing reference-workload resources".to_string(),
            });
            continue;
        };
        if cycles_for(
            row,
            reference_workload(seat),
            settings,
            &baseline(TimeBasis::Pooled),
            runtime_multiplier,
            model_cost_multiplier,
        )
        .is_none()
        {
            excluded.push(ExcludedCandidate {
                row_index,
                reason: "missing pooled-time reference resources".to_string(),
            });
            continue;
        }
        let policy_start = policies.len();
        for (index, cycle) in cycles.into_iter().enumerate() {
            let nominal = cycle_capacity(cycle, tier, settings, range) * (1.0 - cycle.unfinished);
            if tier == Tier::Api || nominal > 0.0 || cycle.model_spend == 0.0 {
                policies.push(Policy {
                    row_index,
                    competence,
                    range,
                    calls,
                    limit: index + 1,
                    cycle,
                    nominal,
                });
            }
        }
        if policies.len() == policy_start {
            excluded.push(ExcludedCandidate {
                row_index,
                reason: "no reference capacity under this allowance".to_string(),
            });
        }
    }
    let scenario_defs = scenarios(settings);
    let values: Vec<Vec<f64>> = scenario_defs
        .iter()
        .map(|assumptions| {
            let mut cached: Vec<Option<[Cycle; 64]>> = vec![None; rows.len()];
            for policy in &policies {
                if cached[policy.row_index].is_none() {
                    cached[policy.row_index] = cycles_for(
                        &rows[policy.row_index],
                        reference_workload(seat),
                        settings,
                        assumptions,
                        adjustment
                            .filter(|value| value.row_index == policy.row_index)
                            .map(|value| value.runtime_multiplier)
                            .unwrap_or(1.0),
                        adjustment
                            .filter(|value| value.row_index == policy.row_index)
                            .map(|value| value.model_cost_multiplier)
                            .unwrap_or(1.0),
                    );
                }
            }
            policies
                .iter()
                .map(|policy| {
                    let cycles = cached[policy.row_index]
                        .as_ref()
                        .expect("eligible policy has complete scenario resources");
                    let range = if policy.calls {
                        (
                            policy.range.0 * assumptions.model_cost,
                            policy.range.1 * assumptions.model_cost,
                        )
                    } else {
                        policy.range
                    };
                    tasks(
                        cycles[policy.limit - 1],
                        tier,
                        settings,
                        range,
                        assumptions.allowance_high,
                    )
                })
                .collect()
        })
        .collect();

    let meets_floor = |policy: &Policy, floor: Option<f64>| {
        floor.is_none_or(|floor| policy.competence.is_some_and(|value| value >= floor))
    };
    let choose = |floor: Option<f64>| -> Option<Evaluation> {
        let feasible: Vec<usize> = policies
            .iter()
            .enumerate()
            .filter_map(|(index, policy)| (meets_floor(policy, floor)).then_some(index))
            .collect();
        if feasible.is_empty() {
            return None;
        }
        let best_capacity: Vec<f64> = values
            .iter()
            .map(|scenario| {
                feasible
                    .iter()
                    .map(|index| scenario[*index])
                    .max_by(f64::total_cmp)
                    .unwrap_or(0.0)
            })
            .collect();
        let capacity: Vec<f64> = policies
            .iter()
            .enumerate()
            .map(|(index, policy)| {
                if !meets_floor(policy, floor) {
                    return f64::INFINITY;
                }
                values
                    .iter()
                    .zip(&best_capacity)
                    .map(|(scenario, best)| {
                        if *best == 0.0 {
                            0.0
                        } else {
                            1.0 - scenario[index] / best
                        }
                    })
                    .fold(0.0, f64::max)
            })
            .collect();
        let selected = feasible
            .into_iter()
            .min_by(|left, right| policy_order(*left, *right, &policies, &capacity, rows))?;
        Some(Evaluation {
            selected,
            capacity,
            best_capacity,
        })
    };

    let mut thresholds = if include_frontier {
        vec![0.0]
    } else {
        Vec::new()
    };
    if include_frontier {
        thresholds.extend(policies.iter().filter_map(|policy| policy.competence));
        thresholds.sort_by(f64::total_cmp);
        thresholds.dedup_by(|left, right| left.total_cmp(right).is_eq());
    }
    let points = thresholds
        .into_iter()
        .filter_map(|floor| {
            let Evaluation {
                selected, capacity, ..
            } = choose(Some(floor))?;
            let policy = &policies[selected];
            let low = values
                .iter()
                .map(|scenario| scenario[selected])
                .fold(f64::INFINITY, f64::min);
            let high = values
                .iter()
                .map(|scenario| scenario[selected])
                .fold(f64::NEG_INFINITY, f64::max);
            let cycles = cycle_capacity(policy.cycle, tier, settings, policy.range);
            Some(FrontierPoint {
                competence_floor: floor,
                row_index: policy.row_index,
                competence: policy.competence,
                attempt_limit: policy.limit,
                tasks_per_week: policy.nominal,
                assisted_tasks_per_week: cycles * policy.cycle.unfinished,
                escalation_hours_per_week: cycles * policy.cycle.escalation_seconds / 3600.0,
                tasks_low: low,
                tasks_high: high,
                capacity_shortfall: capacity[selected],
            })
        })
        .collect();
    let configured_floor = settings
        .competence_floors
        .get(seat.name())
        .copied()
        .flatten();
    let floor = configured_floor;
    if configured_floor.is_some() {
        for row_index in policies
            .iter()
            .filter(|policy| !meets_floor(policy, floor))
            .map(|policy| policy.row_index)
            .collect::<BTreeSet<_>>()
        {
            excluded.push(ExcludedCandidate {
                row_index,
                reason: if competence(&rows[row_index], seat).is_none() {
                    "missing required competence evidence"
                } else {
                    "below configured competence floor"
                }
                .to_string(),
            });
        }
    }
    let frontier = SeatTierFrontier {
        seat,
        tier,
        points,
        excluded,
    };
    let Some(Evaluation {
        selected,
        capacity,
        best_capacity,
    }) = choose(floor)
    else {
        return (None, frontier);
    };

    let make_pick = |index: usize| {
        let policy = &policies[index];
        let cycle = policy.cycle;
        let low = values
            .iter()
            .map(|scenario| scenario[index])
            .fold(f64::INFINITY, f64::min);
        let high = values
            .iter()
            .map(|scenario| scenario[index])
            .fold(f64::NEG_INFINITY, f64::max);
        let time = settings.agent_hours * 3600.0 / cycle.wall;
        let mean_amount = (policy.range.0 + policy.range.1) / 2.0;
        let quota = (tier != Tier::Api && cycle.model_spend > 0.0)
            .then_some(mean_amount / cycle.model_spend);
        let a_star_hours = quota.map(|value| value * cycle.wall / 3600.0);
        let cycles_per_week = cycle_capacity(cycle, tier, settings, policy.range);
        CandidatePick {
            row_index: policy.row_index,
            competence: policy.competence,
            competence_floor: configured_floor,
            capacity_shortfall: capacity[index],
            minutes_per_task: cycle.wall / 60.0,
            cost_per_task: cycle.spend,
            attempt_limit: policy.limit,
            model_cost_per_task: cycle.model_spend,
            escalation_cost_per_task: cycle.escalation_spend,
            escalation_rate: cycle.unfinished,
            cycles_per_week,
            assisted_tasks_per_week: cycles_per_week * cycle.unfinished,
            escalation_hours_per_week: cycles_per_week * cycle.escalation_seconds / 3600.0,
            agent_hours_per_week: cycles_per_week * cycle.agent_seconds / 3600.0,
            tasks_per_week: policy.nominal,
            tasks_low: low,
            tasks_high: high,
            streams_star: quota.map(|value| value / time),
            a_star_hours,
            util_pct: a_star_hours.map(|hours| {
                if hours == 0.0 {
                    0.0
                } else {
                    100.0 * (settings.agent_hours / hours).min(1.0)
                }
            }),
            monthly_price: subscription_price(&rows[policy.row_index], tier, settings),
        }
    };
    let row_indices: BTreeSet<usize> = policies
        .iter()
        .filter(|policy| meets_floor(policy, floor))
        .map(|policy| policy.row_index)
        .collect();
    let mut row_best: Vec<usize> = row_indices
        .into_iter()
        .filter_map(|row_index| {
            policies
                .iter()
                .enumerate()
                .filter_map(|(index, policy)| {
                    (policy.row_index == row_index && meets_floor(policy, floor)).then_some(index)
                })
                .min_by(|left, right| policy_order(*left, *right, &policies, &capacity, rows))
        })
        .collect();
    row_best.sort_by(|left, right| policy_order(*left, *right, &policies, &capacity, rows));
    let top = row_best.into_iter().map(make_pick).collect();
    let first = make_pick(selected);
    let scenarios = scenario_defs
        .iter()
        .zip(&values)
        .enumerate()
        .map(|(scenario_index, (definition, scenario))| {
            let winner = policies
                .iter()
                .enumerate()
                .filter_map(|(index, policy)| (meets_floor(policy, floor)).then_some(index))
                .max_by(|left, right| {
                    scenario[*left]
                        .total_cmp(&scenario[*right])
                        .then_with(|| {
                            policies[*left]
                                .competence
                                .partial_cmp(&policies[*right].competence)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .then_with(|| policies[*right].limit.cmp(&policies[*left].limit))
                        .then_with(|| {
                            rows[policies[*right].row_index]
                                .display_name()
                                .cmp(&rows[policies[*left].row_index].display_name())
                        })
                        .then_with(|| {
                            rows[policies[*right].row_index]
                                .harness
                                .cmp(&rows[policies[*left].row_index].harness)
                        })
                        .then_with(|| {
                            rows[policies[*right].row_index]
                                .effort
                                .cmp(&rows[policies[*left].row_index].effort)
                        })
                })
                .unwrap_or(selected);
            ScenarioPick {
                name: definition.name.clone(),
                row_index: policies[winner].row_index,
                attempt_limit: policies[winner].limit,
                capacity_shortfall: if best_capacity[scenario_index] == 0.0 {
                    0.0
                } else {
                    1.0 - scenario[selected] / best_capacity[scenario_index]
                },
                tasks_per_week: scenario[winner],
                selected_tasks_per_week: scenario[selected],
            }
        })
        .collect();
    (
        Some(Pick {
            row_index: first.row_index,
            competence: first.competence,
            competence_floor: first.competence_floor,
            capacity_shortfall: first.capacity_shortfall,
            minutes_per_task: first.minutes_per_task,
            cost_per_task: first.cost_per_task,
            attempt_limit: first.attempt_limit,
            model_cost_per_task: first.model_cost_per_task,
            escalation_cost_per_task: first.escalation_cost_per_task,
            escalation_rate: first.escalation_rate,
            cycles_per_week: first.cycles_per_week,
            assisted_tasks_per_week: first.assisted_tasks_per_week,
            escalation_hours_per_week: first.escalation_hours_per_week,
            agent_hours_per_week: first.agent_hours_per_week,
            tasks_per_week: first.tasks_per_week,
            tasks_low: first.tasks_low,
            tasks_high: first.tasks_high,
            streams_star: first.streams_star,
            a_star_hours: first.a_star_hours,
            util_pct: first.util_pct,
            monthly_price: first.monthly_price,
            scenarios,
            top,
        }),
        frontier,
    )
}

pub fn rank_seat(
    rows: &[Row],
    seat: Seat,
    tier: Tier,
    settings: &Settings,
    adjustment: Option<ResourceAdjustment>,
) -> Option<Pick> {
    if adjustment.is_some_and(|value| {
        value.row_index >= rows.len()
            || !value.runtime_multiplier.is_finite()
            || !(0.000_001..=1_000_000.0).contains(&value.runtime_multiplier)
            || !value.model_cost_multiplier.is_finite()
            || !(0.000_001..=1_000_000.0).contains(&value.model_cost_multiplier)
            || !value.allowance_multiplier.is_finite()
            || !(0.000_001..=1_000_000.0).contains(&value.allowance_multiplier)
    }) {
        return None;
    }
    analyze(rows, seat, tier, settings, adjustment, false).0
}

pub fn score(
    mut rows: Vec<Row>,
    settings: &Settings,
    cache_state: CacheState,
    source_fetched_at: String,
) -> Table {
    for row in &mut rows {
        row.retry = crate::retry::resolve(row, settings.rho_override);
    }
    rows.sort_by(|left, right| {
        let wall = |row: &Row| {
            implementation_cycle(row, settings)
                .map(|cycle| cycle.wall)
                .unwrap_or(f64::INFINITY)
        };
        let rate = |row: &Row| {
            implementation_cycle(row, settings)
                .map(|cycle| (1.0 - cycle.unfinished) / cycle.wall)
                .unwrap_or(0.0)
        };
        rate(right)
            .total_cmp(&rate(left))
            .then_with(|| wall(left).total_cmp(&wall(right)))
    });
    let analyses: Vec<_> = Seat::ALL
        .into_iter()
        .flat_map(|seat| Tier::ALL.into_iter().map(move |tier| (seat, tier)))
        .map(|(seat, tier)| {
            let (pick, frontier) = analyze(&rows, seat, tier, settings, None, true);
            (SeatTierPick { seat, tier, pick }, frontier)
        })
        .collect();
    let (picks, frontiers) = analyses.into_iter().unzip();
    let mut table = Table {
        picks,
        frontiers,
        rows,
        generated_at: OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_default(),
        source_fetched_at,
        cache_state,
        plan_comparisons: Vec::new(),
    };
    table.plan_comparisons = crate::comparison::plan_comparisons(&table);
    table
}
