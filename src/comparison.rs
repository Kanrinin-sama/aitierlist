use serde::{Deserialize, Serialize};

use crate::engine::{Allowance, ResourceAdjustment, plan_for, rank_seat};
use crate::settings::Settings;
use crate::types::{Pick, Row, Seat, Table, Tier};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanComparison {
    pub seat: Seat,
    pub left_tier: Tier,
    pub right_tier: Tier,
    pub left_price: Option<f64>,
    pub right_price: Option<f64>,
    pub left_competence: f64,
    pub right_competence: f64,
    pub left_autonomous_tasks_per_week: f64,
    pub right_autonomous_tasks_per_week: f64,
    pub worst_capacity_delta: f64,
    pub best_capacity_delta: f64,
    pub dominant_tier: Option<Tier>,
    pub equivalent: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComparisonPick {
    pub row_index: usize,
    pub attempt_limit: usize,
    pub competence: f64,
    pub autonomous_tasks_per_week: f64,
    pub competence_shortfall: f64,
    pub capacity_shortfall: f64,
    pub worst_shortfall: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BindingShortfall {
    Competence,
    Capacity,
    Balanced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImprovementStatus {
    Found,
    AlreadySelected,
    NotReached,
    NotApplicable,
    Ineligible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImprovementThreshold {
    pub status: ImprovementStatus,
    pub applied_factor: Option<f64>,
    pub winning: Option<ComparisonPick>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CounterfactualReport {
    pub seat: Seat,
    pub tier: Tier,
    pub target_row_index: usize,
    pub baseline: Option<ComparisonPick>,
    pub target: Option<ComparisonPick>,
    pub binding_shortfall: Option<BindingShortfall>,
    pub runtime: ImprovementThreshold,
    pub vendor_usage_cost: ImprovementThreshold,
    pub allowance: ImprovementThreshold,
}

fn same(left: f64, right: f64) -> bool {
    left.total_cmp(&right).is_eq()
}

fn scenario_capacities(pick: &Pick) -> Vec<(&str, f64)> {
    pick.scenarios
        .iter()
        .map(|scenario| (scenario.name.as_str(), scenario.selected_tasks_per_week))
        .collect()
}

pub fn plan_comparisons(table: &Table) -> Vec<PlanComparison> {
    let tiers = [Tier::T200, Tier::T100, Tier::T20];
    let mut comparisons = Vec::new();
    for seat in Seat::ALL {
        for left_index in 0..tiers.len() {
            for right_index in (left_index + 1)..tiers.len() {
                let left_tier = tiers[left_index];
                let right_tier = tiers[right_index];
                let (Some(left), Some(right)) = (
                    table.get_pick(seat, left_tier),
                    table.get_pick(seat, right_tier),
                ) else {
                    continue;
                };
                if left.scenarios.is_empty()
                    || left.scenarios.len() != right.scenarios.len()
                    || !left.scenarios.iter().all(|left_scenario| {
                        right
                            .scenarios
                            .iter()
                            .any(|right_scenario| right_scenario.name == left_scenario.name)
                    })
                    || !right.scenarios.iter().all(|right_scenario| {
                        left.scenarios
                            .iter()
                            .any(|left_scenario| left_scenario.name == right_scenario.name)
                    })
                {
                    continue;
                }
                let left_scenarios = scenario_capacities(left);
                let deltas: Vec<f64> = left_scenarios
                    .iter()
                    .filter_map(|(name, left_capacity)| {
                        right
                            .scenarios
                            .iter()
                            .find(|scenario| scenario.name == *name)
                            .map(|scenario| scenario.selected_tasks_per_week - left_capacity)
                    })
                    .collect();
                if deltas.is_empty() {
                    continue;
                }
                let worst_capacity_delta = deltas.iter().copied().fold(f64::INFINITY, f64::min);
                let best_capacity_delta = deltas.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                let capacities_equal = deltas.iter().all(|delta| same(*delta, 0.0));
                let prices_equal = matches!(
                    (left.monthly_price, right.monthly_price),
                    (Some(left_price), Some(right_price)) if same(left_price, right_price)
                );
                let competence_equal = same(left.competence, right.competence);
                let equivalent = prices_equal && competence_equal && capacities_equal;
                let left_dominates = matches!(
                    (left.monthly_price, right.monthly_price),
                    (Some(left_price), Some(right_price)) if left_price <= right_price
                ) && left.competence >= right.competence
                    && deltas.iter().all(|delta| *delta <= 0.0)
                    && (!prices_equal || !competence_equal || !capacities_equal);
                let right_dominates = matches!(
                    (left.monthly_price, right.monthly_price),
                    (Some(left_price), Some(right_price)) if right_price <= left_price
                ) && right.competence >= left.competence
                    && deltas.iter().all(|delta| *delta >= 0.0)
                    && (!prices_equal || !competence_equal || !capacities_equal);
                let dominant_tier = if left_dominates {
                    Some(left_tier)
                } else if right_dominates {
                    Some(right_tier)
                } else {
                    None
                };
                comparisons.push(PlanComparison {
                    seat,
                    left_tier,
                    right_tier,
                    left_price: left.monthly_price,
                    right_price: right.monthly_price,
                    left_competence: left.competence,
                    right_competence: right.competence,
                    left_autonomous_tasks_per_week: left.tasks_per_week,
                    right_autonomous_tasks_per_week: right.tasks_per_week,
                    worst_capacity_delta,
                    best_capacity_delta,
                    dominant_tier,
                    equivalent,
                });
            }
        }
    }
    comparisons
}

fn comparison_pick(pick: &Pick) -> ComparisonPick {
    ComparisonPick {
        row_index: pick.row_index,
        attempt_limit: pick.attempt_limit,
        competence: pick.competence,
        autonomous_tasks_per_week: pick.tasks_per_week,
        competence_shortfall: pick.competence_shortfall,
        capacity_shortfall: pick.capacity_shortfall,
        worst_shortfall: pick.worst_shortfall,
    }
}

fn candidate_pick(pick: &Pick, row_index: usize) -> Option<ComparisonPick> {
    pick.top
        .iter()
        .find(|candidate| candidate.row_index == row_index)
        .map(|candidate| ComparisonPick {
            row_index: candidate.row_index,
            attempt_limit: candidate.attempt_limit,
            competence: candidate.competence,
            autonomous_tasks_per_week: candidate.tasks_per_week,
            competence_shortfall: candidate.competence_shortfall,
            capacity_shortfall: candidate.capacity_shortfall,
            worst_shortfall: candidate.worst_shortfall,
        })
}

fn threshold(status: ImprovementStatus, detail: &str) -> ImprovementThreshold {
    ImprovementThreshold {
        status,
        applied_factor: None,
        winning: None,
        detail: detail.to_string(),
    }
}

fn wins(pick: &Pick, row_index: usize) -> bool {
    pick.row_index == row_index
}

fn reduction_threshold(
    rows: &[Row],
    settings: &Settings,
    seat: Seat,
    tier: Tier,
    row_index: usize,
    runtime: bool,
) -> ImprovementThreshold {
    let endpoint_factor: f64 = 1e-6;
    let adjustment = |factor| ResourceAdjustment {
        row_index,
        runtime_multiplier: if runtime { factor } else { 1.0 },
        model_cost_multiplier: if runtime { 1.0 } else { factor },
        allowance_multiplier: 1.0,
    };
    let mut bracket = None;
    let mut previous_factor = 1.0;
    for step in 1..=24 {
        let factor = (endpoint_factor.ln() * f64::from(step) / 24.0).exp();
        let Some(trial) = rank_seat(rows, seat, tier, settings, Some(adjustment(factor))) else {
            continue;
        };
        if wins(&trial, row_index) {
            bracket = Some((factor, previous_factor, trial));
            break;
        }
        previous_factor = factor;
    }
    let Some((mut winning, mut losing, mut winning_pick)) = bracket else {
        return threshold(
            ImprovementStatus::NotReached,
            "No winning change found at sampled reductions up to 99.9999%",
        );
    };
    for _ in 0..32 {
        let midpoint = (winning + losing) / 2.0;
        let Some(trial) = rank_seat(rows, seat, tier, settings, Some(adjustment(midpoint))) else {
            losing = midpoint;
            continue;
        };
        if wins(&trial, row_index) {
            winning = midpoint;
            winning_pick = trial;
        } else {
            losing = midpoint;
        }
    }
    ImprovementThreshold {
        status: ImprovementStatus::Found,
        applied_factor: Some(winning),
        winning: Some(comparison_pick(&winning_pick)),
        detail: if runtime {
            "Estimated winning runtime change; not a guaranteed global minimum"
        } else {
            "Estimated winning vendor usage cost change; not a guaranteed global minimum"
        }
        .to_string(),
    }
}

fn allowance_threshold(
    rows: &[Row],
    settings: &Settings,
    seat: Seat,
    tier: Tier,
    row_index: usize,
) -> ImprovementThreshold {
    if tier == Tier::Api {
        return threshold(
            ImprovementStatus::NotApplicable,
            "API selection has no subscription allowance",
        );
    }
    if !crate::engine::has_positive_allowance(&rows[row_index], tier, settings) {
        return threshold(
            ImprovementStatus::NotApplicable,
            "Allowance multiplication needs a positive starting allowance",
        );
    }
    let adjustment = |factor| ResourceAdjustment {
        row_index,
        runtime_multiplier: 1.0,
        model_cost_multiplier: 1.0,
        allowance_multiplier: factor,
    };
    let endpoint_factor: f64 = 1_000_000.0;
    let mut bracket = None;
    let mut previous_log = 0.0;
    for step in 1..=24 {
        let factor_log = endpoint_factor.ln() * f64::from(step) / 24.0;
        let factor = factor_log.exp();
        let Some(trial) = rank_seat(rows, seat, tier, settings, Some(adjustment(factor))) else {
            continue;
        };
        if wins(&trial, row_index) {
            bracket = Some((factor_log, previous_log, trial));
            break;
        }
        previous_log = factor_log;
    }
    let Some((mut winning_log, mut losing_log, mut winning_pick)) = bracket else {
        return threshold(
            ImprovementStatus::NotReached,
            "No winning change found at sampled allowances up to 1,000,000x",
        );
    };
    for _ in 0..32 {
        let midpoint_log = (winning_log + losing_log) / 2.0;
        let midpoint = midpoint_log.exp();
        let Some(trial) = rank_seat(rows, seat, tier, settings, Some(adjustment(midpoint))) else {
            losing_log = midpoint_log;
            continue;
        };
        if wins(&trial, row_index) {
            winning_log = midpoint_log;
            winning_pick = trial;
        } else {
            losing_log = midpoint_log;
        }
    }
    ImprovementThreshold {
        status: ImprovementStatus::Found,
        applied_factor: Some(winning_log.exp()),
        winning: Some(comparison_pick(&winning_pick)),
        detail: "Estimated winning allowance change; not a guaranteed global minimum".to_string(),
    }
}

pub fn compare_candidate(
    rows: &[Row],
    settings: &Settings,
    seat: Seat,
    tier: Tier,
    row_index: usize,
) -> CounterfactualReport {
    let ineligible_report = |detail: &str| CounterfactualReport {
        seat,
        tier,
        target_row_index: row_index,
        baseline: None,
        target: None,
        binding_shortfall: None,
        runtime: threshold(ImprovementStatus::Ineligible, detail),
        vendor_usage_cost: threshold(ImprovementStatus::Ineligible, detail),
        allowance: threshold(ImprovementStatus::Ineligible, detail),
    };
    if row_index >= rows.len() {
        return ineligible_report("Target row does not exist");
    }
    let Some(baseline_pick) = rank_seat(rows, seat, tier, settings, None) else {
        return ineligible_report("No eligible seat policy exists");
    };
    let baseline = comparison_pick(&baseline_pick);
    let Some(target) = candidate_pick(&baseline_pick, row_index) else {
        return CounterfactualReport {
            seat,
            tier,
            target_row_index: row_index,
            baseline: Some(baseline),
            target: None,
            binding_shortfall: None,
            runtime: threshold(
                ImprovementStatus::Ineligible,
                "Target is not eligible for this seat and tier",
            ),
            vendor_usage_cost: threshold(
                ImprovementStatus::Ineligible,
                "Target is not eligible for this seat and tier",
            ),
            allowance: threshold(
                ImprovementStatus::Ineligible,
                "Target is not eligible for this seat and tier",
            ),
        };
    };
    let binding_shortfall = if same(target.competence_shortfall, target.capacity_shortfall) {
        BindingShortfall::Balanced
    } else if target.competence_shortfall > target.capacity_shortfall {
        BindingShortfall::Competence
    } else {
        BindingShortfall::Capacity
    };
    let cost_not_applicable = tier == Tier::Api
        || (settings
            .vendor_overrides
            .get(&rows[row_index].vendor)
            .copied()
            .flatten()
            .is_none()
            && matches!(
                plan_for(
                    &rows[row_index].vendor,
                    match tier {
                        Tier::Api => -1.0,
                        Tier::T200 => settings.plan_prices.t200,
                        Tier::T100 => settings.plan_prices.t100,
                        Tier::T20 => settings.plan_prices.t20,
                    }
                ),
                Some(Allowance::Calls(_))
            ));
    let cost_not_applicable_threshold = || {
        threshold(
            ImprovementStatus::NotApplicable,
            if tier == Tier::Api {
                "API selection is not constrained by subscription allowance"
            } else {
                "Native call quotas scale with the modeled call price"
            },
        )
    };
    if baseline_pick.row_index == row_index {
        let already_selected = || {
            let mut value = threshold(
                ImprovementStatus::AlreadySelected,
                "Candidate is already the selected policy",
            );
            value.applied_factor = Some(1.0);
            value.winning = Some(comparison_pick(&baseline_pick));
            value
        };
        return CounterfactualReport {
            seat,
            tier,
            target_row_index: row_index,
            baseline: Some(baseline),
            target: Some(target),
            binding_shortfall: Some(binding_shortfall),
            runtime: already_selected(),
            vendor_usage_cost: if cost_not_applicable {
                cost_not_applicable_threshold()
            } else {
                already_selected()
            },
            allowance: if tier == Tier::Api {
                threshold(
                    ImprovementStatus::NotApplicable,
                    "API selection has no subscription allowance",
                )
            } else {
                already_selected()
            },
        };
    }
    CounterfactualReport {
        seat,
        tier,
        target_row_index: row_index,
        baseline: Some(baseline),
        target: Some(target),
        binding_shortfall: Some(binding_shortfall),
        runtime: reduction_threshold(rows, settings, seat, tier, row_index, true),
        vendor_usage_cost: if cost_not_applicable {
            cost_not_applicable_threshold()
        } else {
            reduction_threshold(rows, settings, seat, tier, row_index, false)
        },
        allowance: allowance_threshold(rows, settings, seat, tier, row_index),
    }
}
