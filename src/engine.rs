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
const REVIEWER: [(Benchmark, f64); 3] = [
    (Benchmark::Qna, 0.5),
    (Benchmark::Gpqa, 0.25),
    (Benchmark::Hle, 0.25),
];
const SANITY: [(Benchmark, f64); 1] = [(Benchmark::Qna, 1.0)];
const COMPREHENSION: [(Benchmark, f64); 2] = [(Benchmark::Lcr, 0.5), (Benchmark::Qna, 0.5)];
const QNA: [(Benchmark, f64); 1] = [(Benchmark::Qna, 1.0)];

fn coding_workload(row: &Row) -> Option<Vec<(Benchmark, f64)>> {
    let swe = row.benchmark_tasks(Benchmark::Swe)? as f64;
    let terminal = row.benchmark_tasks(Benchmark::Terminal)? as f64;
    let total = swe + terminal;
    Some(vec![
        (Benchmark::Swe, swe / total),
        (Benchmark::Terminal, terminal / total),
    ])
}

fn pass(row: &Row, benchmark: Benchmark) -> Option<f64> {
    let value = match benchmark {
        Benchmark::Swe => row.swe,
        Benchmark::Terminal => row.term,
        Benchmark::Qna => row.qna,
        Benchmark::Gpqa => row.gpqa,
        Benchmark::Hle => row.hle,
        Benchmark::Lcr => row.lcr,
        Benchmark::Omniscience => row.omniscience_accuracy,
    }?;
    (value.is_finite() && (0.0..=1.0).contains(&value))
        .then(|| row.retry.adjusted_pass(benchmark, value))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RoleEvidenceLevel {
    ExactHarness,
    ModelLevel,
    CrossHarnessProxy,
    Unknown,
}

pub struct RoleEvidenceComponent {
    pub name: &'static str,
    pub benchmark: Option<Benchmark>,
    pub value: Option<f64>,
    pub weight: f64,
    pub level: RoleEvidenceLevel,
    pub source_id: Option<String>,
    pub observation_id: Option<String>,
    pub basis: String,
}

#[derive(Clone, Copy)]
pub struct ScoreSensitivity {
    pub nominal: Option<f64>,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub incomplete: bool,
}

pub(crate) fn weighted_geometric(values: &[(f64, f64)]) -> Option<f64> {
    if values.iter().any(|(value, weight)| {
        !value.is_finite() || !(0.0..=1.0).contains(value) || !weight.is_finite() || *weight < 0.0
    }) {
        return None;
    }
    if values
        .iter()
        .any(|(value, weight)| *weight > 0.0 && *value == 0.0)
    {
        return Some(0.0);
    }
    let mut logarithm = 0.0;
    let mut total_weight = 0.0;
    for &(value, weight) in values {
        if weight == 0.0 {
            continue;
        }
        logarithm += weight * value.ln();
        total_weight += weight;
    }
    (total_weight > 0.0).then(|| (logarithm / total_weight).exp())
}

pub fn orchestrator_capability(row: &Row) -> Option<f64> {
    let intelligence = row.smart?;
    weighted_geometric(&[(intelligence, 1.0)])
}

pub fn competence(row: &Row, seat: Seat) -> Option<f64> {
    if seat == Seat::Orchestrator {
        return orchestrator_capability(row);
    }
    if seat == Seat::NetResearch {
        return None;
    }
    let values = competence_profile(row, seat)?
        .into_iter()
        .map(|(benchmark, weight)| Some((pass(row, benchmark)?, weight)))
        .collect::<Option<Vec<_>>>()?;
    weighted_geometric(&values)
}

pub fn role_evidence(row: &Row, seat: Seat) -> Option<Vec<RoleEvidenceComponent>> {
    if seat == Seat::NetResearch {
        return None;
    }
    let profile: Vec<(Option<Benchmark>, &'static str, Option<f64>, f64)> =
        if seat == Seat::Orchestrator {
            return orchestrator_role_evidence(row);
        } else {
            competence_profile(row, seat)?
                .into_iter()
                .map(|(benchmark, weight)| {
                    (
                        Some(benchmark),
                        benchmark.name(),
                        pass(row, benchmark),
                        weight,
                    )
                })
                .collect()
        };
    Some(
        profile
            .into_iter()
            .map(|(benchmark, name, value, weight)| {
                let family = benchmark
                    .map(benchmark_family)
                    .unwrap_or("intelligence-index");
                let projection = row
                    .benchmark_evidence
                    .iter()
                    .find(|projection| projection.family == family);
                let level = projection.map_or(RoleEvidenceLevel::Unknown, |projection| {
                    match projection.transfer {
                        crate::evidence::TransferKind::ExactHarness => {
                            RoleEvidenceLevel::ExactHarness
                        }
                        crate::evidence::TransferKind::ModelLevel => RoleEvidenceLevel::ModelLevel,
                        crate::evidence::TransferKind::CrossHarnessProxy => {
                            RoleEvidenceLevel::CrossHarnessProxy
                        }
                    }
                });
                RoleEvidenceComponent {
                    name,
                    benchmark,
                    value: value.filter(|value| value.is_finite() && (0.0..=1.0).contains(value)),
                    weight,
                    level,
                    source_id: projection.map(|projection| projection.source.source_id.clone()),
                    observation_id: projection.map(|projection| projection.observation_id.clone()),
                    basis: projection.map_or_else(
                        || "No linked benchmark observation".to_owned(),
                        |projection| projection.source.url.clone(),
                    ),
                }
            })
            .collect(),
    )
}

pub fn orchestrator_role_evidence(row: &Row) -> Option<Vec<RoleEvidenceComponent>> {
    let components = vec![
        (None, "Intelligence Index", row.smart, 1.0),
        (
            Some(Benchmark::Lcr),
            Benchmark::Lcr.name(),
            pass(row, Benchmark::Lcr),
            0.0,
        ),
        (
            Some(Benchmark::Hle),
            Benchmark::Hle.name(),
            pass(row, Benchmark::Hle),
            0.0,
        ),
    ];
    Some(
        components
            .into_iter()
            .map(|(benchmark, name, value, weight)| {
                let family = benchmark
                    .map(benchmark_family)
                    .unwrap_or("intelligence-index");
                let projection = row
                    .benchmark_evidence
                    .iter()
                    .find(|projection| projection.family == family);
                RoleEvidenceComponent {
                    name,
                    benchmark,
                    value: value.filter(|value| value.is_finite() && (0.0..=1.0).contains(value)),
                    weight,
                    level: if benchmark.is_none() && value.is_some() {
                        RoleEvidenceLevel::ModelLevel
                    } else {
                        projection.map_or(RoleEvidenceLevel::Unknown, |projection| match projection
                            .transfer
                        {
                            crate::evidence::TransferKind::ExactHarness => {
                                RoleEvidenceLevel::ExactHarness
                            }
                            crate::evidence::TransferKind::ModelLevel => {
                                RoleEvidenceLevel::ModelLevel
                            }
                            crate::evidence::TransferKind::CrossHarnessProxy => {
                                RoleEvidenceLevel::CrossHarnessProxy
                            }
                        })
                    },
                    source_id: projection.map(|projection| projection.source.source_id.clone()),
                    observation_id: projection.map(|projection| projection.observation_id.clone()),
                    basis: if benchmark.is_none() {
                        "Orchestration competence score (Intelligence Index)".to_owned()
                    } else {
                        projection.map_or_else(
                            || "No linked benchmark observation".to_owned(),
                            |projection| projection.source.url.clone(),
                        )
                    },
                }
            })
            .collect(),
    )
}

pub fn competence_sensitivity(
    row: &Row,
    seat: Seat,
    settings: &Settings,
) -> Option<ScoreSensitivity> {
    let components = role_evidence(row, seat)?;
    component_sensitivity(&components, settings)
}

pub fn orchestrator_capability_sensitivity(
    row: &Row,
    settings: &Settings,
) -> Option<ScoreSensitivity> {
    let components = orchestrator_role_evidence(row)?;
    component_sensitivity(&components, settings)
}

fn component_sensitivity(
    components: &[RoleEvidenceComponent],
    settings: &Settings,
) -> Option<ScoreSensitivity> {
    let span = (settings.assumption_span_pct / 100.0).clamp(0.0, 1.0);
    let incomplete = components
        .iter()
        .any(|component| component.weight > 0.0 && component.value.is_none());
    let nominal = (!incomplete)
        .then(|| {
            let values: Vec<_> = components
                .iter()
                .map(|component| (component.value.unwrap_or_default(), component.weight))
                .collect();
            weighted_geometric(&values)
        })
        .flatten();
    if incomplete {
        return Some(ScoreSensitivity {
            nominal: None,
            low: None,
            high: None,
            incomplete: true,
        });
    }
    let low_values: Vec<_> = components
        .iter()
        .map(|component| {
            let value = component.value.unwrap_or_default() * (1.0 - span);
            (value, component.weight)
        })
        .collect();
    let high_values: Vec<_> = components
        .iter()
        .map(|component| {
            (
                (component.value.unwrap_or_default() * (1.0 + span)).min(1.0),
                component.weight,
            )
        })
        .collect();
    let low = weighted_geometric(&low_values)?;
    let high = weighted_geometric(&high_values)?;
    Some(ScoreSensitivity {
        nominal,
        low: Some(low),
        high: Some(high),
        incomplete,
    })
}

pub fn weakest_evidence_level(components: &[RoleEvidenceComponent]) -> RoleEvidenceLevel {
    components
        .iter()
        .filter(|component| component.weight > 0.0)
        .map(|component| component.level)
        .min_by_key(|level| match level {
            RoleEvidenceLevel::Unknown => 0,
            RoleEvidenceLevel::CrossHarnessProxy => 1,
            RoleEvidenceLevel::ModelLevel => 2,
            RoleEvidenceLevel::ExactHarness => 3,
        })
        .unwrap_or(RoleEvidenceLevel::Unknown)
}

fn benchmark_family(benchmark: Benchmark) -> &'static str {
    match benchmark {
        Benchmark::Swe => "swe",
        Benchmark::Terminal => "terminal",
        Benchmark::Qna => "qna",
        Benchmark::Gpqa => "gpqa",
        Benchmark::Hle => "hle",
        Benchmark::Lcr => "lcr",
        Benchmark::Omniscience => "omniscience",
    }
}

fn competence_profile(row: &Row, seat: Seat) -> Option<Vec<(Benchmark, f64)>> {
    let profile = match seat {
        Seat::Implementer => coding_workload(row)?,
        Seat::Debugger => coding_workload(row)?
            .into_iter()
            .map(|(benchmark, weight)| (benchmark, weight * 0.5))
            .chain([(Benchmark::Gpqa, 0.25), (Benchmark::Hle, 0.25)])
            .collect(),
        Seat::Reviewer => REVIEWER.to_vec(),
        Seat::Sanity => SANITY.to_vec(),
        Seat::Comprehension => COMPREHENSION.to_vec(),
        Seat::NetResearch | Seat::Orchestrator => Vec::new(),
    };
    Some(profile)
}

pub fn reference_workload(row: &Row, seat: Seat) -> Option<Vec<(Benchmark, f64)>> {
    match seat {
        Seat::Implementer | Seat::Debugger => coding_workload(row),
        Seat::Reviewer | Seat::Sanity | Seat::Comprehension => Some(QNA.to_vec()),
        Seat::NetResearch | Seat::Orchestrator => None,
    }
}

pub fn reference_workload_name(seat: Seat) -> &'static str {
    match seat {
        Seat::Implementer | Seat::Debugger => "coding reference workload",
        Seat::NetResearch => "research class benchmark mixture",
        Seat::Orchestrator => "coordination resource reference task",
        _ => "repository Q&A reference workload",
    }
}

#[derive(Clone, Copy)]
pub enum Allowance {
    Usd(f64, f64),
    Calls(f64),
}

fn priced_plan_for(vendor: &str, budget: f64) -> Option<(f64, Allowance)> {
    let plan = crate::subscriptions::plan_for(vendor, budget)?;
    let allowance = if let Some(window) = plan.weekly_window() {
        let (low, high) = window.cap.bounds(0.0);
        Allowance::Usd(low, high)
    } else {
        let window = plan
            .windows
            .iter()
            .find(|window| window.unit == crate::types::CapacityUnit::Requests)?;
        Allowance::Calls(window.cap.bounds(0.0).0)
    };
    Some((plan.monthly_price, allowance))
}

fn weekly_override(vendor: &str, budget: f64, settings: &Settings) -> Option<f64> {
    crate::subscriptions::plan_for(vendor, budget)
        .and_then(|plan| {
            plan.weekly_window()
                .and_then(|window| plan.override_amount(vendor, window, settings))
        })
        .or_else(|| settings.vendor_overrides.get(vendor).copied().flatten())
}

fn capacity_basis(vendor: &str, budget: f64, settings: &Settings) -> &'static str {
    if budget < 0.0 {
        "API; no subscription cap"
    } else if weekly_override(vendor, budget, settings).is_some() {
        "user-override"
    } else if let Some(plan) = crate::subscriptions::plan_for(vendor, budget) {
        plan.weekly_basis(vendor, settings)
    } else {
        "no weekly USD window"
    }
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

fn plan_eligible(row: &Row, plan: Option<Allowance>, budget: f64) -> bool {
    if row.vendor == "anthropic"
        && crate::subscriptions::is_fable(row)
        && crate::subscriptions::plan_for(&row.vendor, budget)
            .is_some_and(|plan| plan.id == "claude-pro")
    {
        return false;
    }
    let Some(plan) = plan else { return false };
    if matches!(plan, Allowance::Calls(amount) if amount <= 0.0) {
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
) -> Option<Vec<Cycle>> {
    let limit = if settings.rho_override.is_some()
        || mix
            .iter()
            .all(|(benchmark, _)| row.retry.repeat_evidence(*benchmark).is_some())
    {
        64
    } else {
        3
    };
    let mut totals = vec![Cycle::default(); limit];
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
        let rho = (settings
            .rho_override
            .or_else(|| row.retry.repeat_evidence(benchmark).map(|value| value.rho))
            .unwrap_or(0.0)
            * assumptions.rho_multiplier)
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

fn role_cycles(
    row: &Row,
    seat: Seat,
    settings: &Settings,
    assumptions: &Assumptions,
    runtime_multiplier: f64,
    model_cost_multiplier: f64,
) -> Option<Vec<Cycle>> {
    if seat == Seat::Orchestrator {
        let model_spend = row.orchestrator_usd? * assumptions.model_cost * model_cost_multiplier;
        let wall = row.orchestrator_seconds? * assumptions.runtime * runtime_multiplier;
        return (model_spend.is_finite() && model_spend >= 0.0 && wall.is_finite() && wall > 0.0)
            .then_some(vec![Cycle {
                wall,
                spend: model_spend,
                model_spend,
                agent_seconds: wall,
                ..Cycle::default()
            }]);
    }
    let workload = reference_workload(row, seat)?;
    cycles_for(
        row,
        &workload,
        settings,
        assumptions,
        runtime_multiplier,
        model_cost_multiplier,
    )
}

pub fn implementation_cycle(row: &Row, settings: &Settings) -> Option<Cycle> {
    let mix = if row.harness == "model" {
        vec![(Benchmark::Terminal, 1.0)]
    } else {
        coding_workload(row)?
    };
    cycles_for(row, &mix, settings, &baseline(TimeBasis::Token), 1.0, 1.0)?
        .into_iter()
        .reduce(|best, next| {
            if (1.0 - next.unfinished) / next.wall > (1.0 - best.unfinished) / best.wall {
                next
            } else {
                best
            }
        })
}

#[derive(Clone, Copy, Default)]
pub(crate) struct DispatchCycle {
    pub success: f64,
    pub nominal_usage: f64,
    pub nominal_seconds: f64,
    pub reserved_usage: f64,
    pub reserved_seconds: f64,
    pub(crate) reference_completion: [Option<f64>; 3],
}

pub(crate) fn dispatch_utility(row: &Row, seat: Seat, cycle: &DispatchCycle) -> Option<f64> {
    if seat == Seat::Orchestrator {
        return competence(row, seat);
    }
    let values = dispatch_components(row, seat, cycle)?;
    weighted_geometric(&values)
}

fn dispatch_components(row: &Row, seat: Seat, cycle: &DispatchCycle) -> Option<Vec<(f64, f64)>> {
    let workload = reference_workload(row, seat)?;
    competence_profile(row, seat)?
        .iter()
        .map(|(benchmark, weight)| {
            let value = if workload.iter().any(|(reference, _)| reference == benchmark) {
                cycle.reference_completion[match benchmark {
                    Benchmark::Swe => 0,
                    Benchmark::Terminal => 1,
                    Benchmark::Qna => 2,
                    _ => return None,
                }]?
            } else {
                pass(row, *benchmark)?
            };
            Some((value, *weight))
        })
        .collect()
}

pub(crate) fn dispatch_utility_sensitivity(
    row: &Row,
    seat: Seat,
    cycle: &DispatchCycle,
    settings: &Settings,
) -> Option<ScoreSensitivity> {
    if seat == Seat::Orchestrator {
        return competence_sensitivity(row, seat, settings);
    }
    let values = dispatch_components(row, seat, cycle)?;
    let nominal = weighted_geometric(&values)?;
    let span = (settings.assumption_span_pct / 100.0).clamp(0.0, 1.0);
    let low_values: Vec<_> = values
        .iter()
        .map(|(value, weight)| (value * (1.0 - span), *weight))
        .collect();
    let high_values: Vec<_> = values
        .iter()
        .map(|(value, weight)| ((value * (1.0 + span)).min(1.0), *weight))
        .collect();
    Some(ScoreSensitivity {
        nominal: Some(nominal),
        low: Some(weighted_geometric(&low_values)?),
        high: Some(weighted_geometric(&high_values)?),
        incomplete: false,
    })
}

pub(crate) fn dispatch_cycles(
    row: &Row,
    seat: Seat,
    settings: &Settings,
) -> Option<Vec<DispatchCycle>> {
    let workload = reference_workload(row, seat)?;
    let limit = 3;
    let mut nominal = vec![Cycle::default(); limit];
    let mut reference_completion = vec![[None; 3]; limit];
    let mut full_usage = 0.0;
    let mut token_seconds = 0.0;
    let mut pooled_seconds = 0.0;
    for (benchmark, weight) in &workload {
        let component = cycles_for(
            row,
            &[(*benchmark, 1.0)],
            settings,
            &baseline(TimeBasis::Token),
            1.0,
            1.0,
        )?;
        let benchmark_index = match benchmark {
            Benchmark::Swe => 0,
            Benchmark::Terminal => 1,
            Benchmark::Qna => 2,
            _ => return None,
        };
        for index in 0..limit {
            nominal[index].unfinished += weight * component[index].unfinished;
            nominal[index].model_spend += weight * component[index].model_spend;
            nominal[index].agent_seconds += weight * component[index].agent_seconds;
            reference_completion[index][benchmark_index] = Some(1.0 - component[index].unfinished);
        }
        let metric = row
            .task_metrics
            .iter()
            .find(|metric| metric.benchmark == *benchmark)?;
        if !metric.pooled_seconds.is_finite() || metric.pooled_seconds <= 0.0 {
            return None;
        }
        full_usage += weight * metric.usd;
        token_seconds += weight * metric.seconds;
        pooled_seconds += weight * metric.pooled_seconds;
    }
    let stress = 1.0 + settings.assumption_span_pct / 100.0;
    Some(
        (0..limit)
            .map(|index| DispatchCycle {
                success: 1.0 - nominal[index].unfinished,
                nominal_usage: nominal[index].model_spend,
                nominal_seconds: nominal[index].agent_seconds,
                reserved_usage: (index + 1) as f64 * full_usage * stress,
                reserved_seconds: (index + 1) as f64 * token_seconds.max(pooled_seconds) * stress,
                reference_completion: reference_completion[index],
            })
            .collect(),
    )
}

pub(crate) fn orchestrator_dispatch_cycle(row: &Row, settings: &Settings) -> Option<DispatchCycle> {
    let cycle = role_cycles(
        row,
        Seat::Orchestrator,
        settings,
        &baseline(TimeBasis::Token),
        1.0,
        1.0,
    )?
    .into_iter()
    .next()?;
    let stress = 1.0 + settings.assumption_span_pct / 100.0;
    Some(DispatchCycle {
        success: 1.0,
        nominal_usage: cycle.model_spend,
        nominal_seconds: cycle.wall,
        reserved_usage: cycle.model_spend * stress,
        reserved_seconds: cycle.wall * stress,
        reference_completion: [None; 3],
    })
}

fn allowance(plan: Option<Allowance>, override_amount: Option<f64>) -> (f64, f64) {
    if let Some(amount) = override_amount {
        return (amount, amount);
    }
    match plan {
        Some(Allowance::Usd(low, high)) => (low, high),
        Some(Allowance::Calls(_)) | None => (0.0, 0.0),
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
    let override_amount = weekly_override(&row.vendor, budget, settings);
    let range = allowance(plan, override_amount);
    range.1 > 0.0
        && (plan_eligible(row, plan, budget) || plan.is_none() && override_amount.is_some())
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
        let Some(competence) = competence(row, seat) else {
            excluded.push(ExcludedCandidate {
                row_index,
                reason: "missing required competence evidence".to_string(),
            });
            continue;
        };
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
        let override_amount = weekly_override(&row.vendor, budget, settings);
        if tier != Tier::Api
            && !plan_eligible(row, plan, budget)
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
        let mut range = allowance(plan, override_amount);
        let range_multiplier = allowance_multiplier;
        range.0 *= range_multiplier;
        range.1 *= range_multiplier;
        let Some(cycles) = role_cycles(
            row,
            seat,
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
        if role_cycles(
            row,
            seat,
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
                    competence: Some(competence),
                    range,
                    calls: false,
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
            let mut cached: Vec<Option<Vec<Cycle>>> = vec![None; rows.len()];
            for policy in &policies {
                if cached[policy.row_index].is_none() {
                    cached[policy.row_index] = role_cycles(
                        &rows[policy.row_index],
                        seat,
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
            allowance_basis: capacity_basis(&rows[policy.row_index].vendor, budget, settings)
                .to_owned(),
            capacity_windows: crate::subscriptions::plan_for(
                &rows[policy.row_index].vendor,
                budget,
            )
            .map(|plan| plan.resolved_windows(&rows[policy.row_index].vendor, settings))
            .unwrap_or_default(),
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
                name: format!(
                    "{} · selected capacity: {} · comparator capacity: {}",
                    definition.name,
                    capacity_basis(&rows[policies[selected].row_index].vendor, budget, settings),
                    capacity_basis(&rows[policies[winner].row_index].vendor, budget, settings)
                ),
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
            allowance_basis: first.allowance_basis,
            capacity_windows: first.capacity_windows,
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
    vendor_filter: Option<&str>,
) -> Table {
    let evidence_catalog =
        crate::upstream::project(&mut rows, settings.allow_cross_harness_benchmark_proxies);
    if let Some(vendor) = vendor_filter {
        rows.retain(|row| row.vendor == vendor);
    }
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
    let portfolio = vendor_filter
        .is_none()
        .then(|| crate::portfolio::allocate(&rows, settings))
        .flatten();
    let research_tiers = crate::portfolio::research_tier_picks(&rows, settings, vendor_filter);
    let mut table = Table {
        portfolio,
        picks,
        frontiers,
        rows,
        research_tiers,
        evidence_catalog,
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
