use crate::engine::{self, DispatchCycle};
use crate::settings::Settings;
use crate::solver::{self, Choice, Problem};
use crate::subscriptions;
use crate::types::{CapacityDemand, CapacityUnit, Row, Seat};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const SEARCH_NODES: usize = 1024;
const CLASS_MIX: [f64; 4] = [0.50, 0.30, 0.15, 0.05];
const ORCHESTRATOR_WELFARE: f64 = 0.24;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkClass {
    Focused,
    Standard,
    Complex,
    Extensive,
}

impl WorkClass {
    pub const ALL: [Self; 4] = [
        Self::Focused,
        Self::Standard,
        Self::Complex,
        Self::Extensive,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Focused => "Focused",
            Self::Standard => "Standard",
            Self::Complex => "Complex",
            Self::Extensive => "Extensive",
        }
    }
    pub fn condition(self) -> &'static str {
        match self {
            Self::Extensive => {
                "Open-ended research, broad system design, high consequence, or unresolved scope"
            }
            Self::Complex => {
                "Interacting decisions, uncertain diagnosis, consequential interfaces, or costly recovery"
            }
            Self::Standard => "Well-specified implementation requiring an independent check",
            Self::Focused => {
                "Bounded question with known sources, observable acceptance, and reversible output"
            }
        }
    }
    pub fn resource_factor(self) -> f64 {
        [0.25, 1.0, 2.0, 4.0][self.index()]
    }
    fn index(self) -> usize {
        self as usize
    }
}

pub struct WorkflowStage {
    pub id: usize,
    pub seat: Seat,
    pub visit: usize,
    pub dependencies: Vec<usize>,
    pub condition: &'static str,
}

fn workflow_stages(class: WorkClass, research: bool) -> Vec<WorkflowStage> {
    let mut stages = Vec::new();
    let mut dependency = None;
    if research {
        stages.push(WorkflowStage {
            id: stages.len(),
            seat: Seat::NetResearch,
            visit: 1,
            dependencies: Vec::new(),
            condition: "Class policy includes one declared research reference visit",
        });
        dependency = Some(stages.len() - 1);
    }
    if matches!(class, WorkClass::Complex | WorkClass::Extensive) {
        stages.push(WorkflowStage {
            id: stages.len(),
            seat: Seat::Comprehension,
            visit: 1,
            dependencies: dependency.into_iter().collect(),
            condition: "Brief and evidence packet accepted",
        });
        dependency = Some(stages.len() - 1);
    }
    stages.push(WorkflowStage {
        id: stages.len(),
        seat: Seat::Implementer,
        visit: 1,
        dependencies: dependency.into_iter().collect(),
        condition: "Required context and evidence accepted",
    });
    let implementation = stages.len() - 1;
    if class == WorkClass::Focused {
        return stages;
    }
    for seat in [Seat::Reviewer, Seat::Sanity] {
        stages.push(WorkflowStage {
            id: stages.len(),
            seat,
            visit: 1,
            dependencies: vec![implementation],
            condition: "Implementation produced immutable output",
        });
    }
    let initial_checks = [stages.len() - 2, stages.len() - 1];
    stages.push(WorkflowStage {
        id: stages.len(),
        seat: Seat::Debugger,
        visit: 1,
        dependencies: initial_checks.to_vec(),
        condition: "Initial implementation blocked or either initial check blocked or completed with rejection, and no other Debugger is running for this task; skip when both checks accepted",
    });
    let repair = stages.len() - 1;
    for seat in [Seat::Reviewer, Seat::Sanity] {
        stages.push(WorkflowStage {
            id: stages.len(),
            seat,
            visit: 2,
            dependencies: vec![repair],
            condition: "Repair produced immutable output; otherwise skip",
        });
    }
    stages
}

fn reserved_visits(class: WorkClass, seat: Seat, research: bool) -> usize {
    workflow_stages(class, research)
        .iter()
        .filter(|stage| stage.seat == seat)
        .count()
}

fn expected_visits(class: WorkClass, seat: Seat, research: bool, repair: f64) -> f64 {
    match seat {
        Seat::Debugger => (class != WorkClass::Focused) as u8 as f64 * repair,
        Seat::Reviewer | Seat::Sanity => {
            (class != WorkClass::Focused) as u8 as f64 * (1.0 + repair)
        }
        _ => reserved_visits(class, seat, research) as f64,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Portfolio {
    #[serde(default = "single_orchestrator")]
    pub orchestrators: usize,
    pub available_hours_per_provider: f64,
    pub total_scheduled_hours: f64,
    pub total_agent_hours: f64,
    pub monthly_price: f64,
    pub roles: Vec<RoleAllocation>,
    pub pools: Vec<PoolUsage>,
    pub limits: Vec<PoolLimit>,
    pub assumptions: Vec<String>,
    pub message: String,
    pub dispatch: DispatchPlan,
    pub math_audit: MathAudit,
    pub conductor: Option<ConductorAllocation>,
    #[serde(default)]
    pub close_choices: Vec<CloseChoice>,
    #[serde(default)]
    pub capability_scenario: Option<CapabilityScenarioReport>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRange {
    pub nominal: f64,
    pub lower: f64,
    pub upper: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassRepairSensitivity {
    pub class: WorkClass,
    pub nominal: f64,
    pub lower: f64,
    pub upper: f64,
    pub basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseChoice {
    pub seat: Seat,
    pub class: Option<WorkClass>,
    pub nominal_row_index: usize,
    pub challenger_row_index: usize,
    pub nominal_utility: f64,
    pub challenger_utility: f64,
    pub challenger_attempt_limit: usize,
    pub challenger_provider_id: String,
    pub challenger_plan_id: String,
    pub challenger_native_harness: String,
    pub scenario_status: String,
    pub scenario_bound: Option<f64>,
    pub scenario_relative_gap: Option<f64>,
    pub scenario_proven: bool,
    pub condition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioAssignment {
    pub seat: Seat,
    pub class: Option<WorkClass>,
    pub row_index: usize,
    pub attempt_limit: usize,
    pub provider_id: String,
    pub plan_id: String,
    pub native_harness: String,
    pub utility: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityScenarioReport {
    pub status: String,
    pub proven: bool,
    pub quality: f64,
    pub baseline_quality: Option<f64>,
    pub bound: Option<f64>,
    pub relative_gap: Option<f64>,
    pub message: String,
    pub assignments: Vec<ScenarioAssignment>,
}

fn single_orchestrator() -> usize {
    1
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MathAudit {
    pub objective: String,
    pub coordination_proxy: String,
    pub role_accounts: Vec<RoleAccountUsage>,
    pub orchestrator_expected_spend_share: f64,
    pub orchestrator_reserved_spend_share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleAccountUsage {
    pub seat: Seat,
    pub provider_id: String,
    pub row_indices: Vec<usize>,
    pub expected_usage: f64,
    pub reserved_usage: f64,
    pub expected_hours: f64,
    pub reserved_hours: f64,
    pub expected_account_share: f64,
    pub reserved_account_share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchPlan {
    #[serde(default = "policy_version")]
    pub policy_version: String,
    pub forecast_changes: usize,
    pub admitted_changes: usize,
    pub deferred_changes: usize,
    pub cadence_hours: Option<f64>,
    pub jobs_per_week: Option<f64>,
    pub repair_incidence: f64,
    pub classes: Vec<WorkClassDemand>,
    pub solver: SolverReport,
    pub executable: bool,
    pub admitted_sequence: Vec<WorkClass>,
    pub reserved_makespan_hours: f64,
    pub schedule: Vec<ScheduledVisit>,
    #[serde(default)]
    pub repair_sensitivity: Vec<ClassRepairSensitivity>,
    #[serde(default)]
    pub service: TeamServiceReport,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamServiceReport {
    pub workload: f64,
    pub team_capability: f64,
    pub quality_adjusted_service: f64,
    pub maximum_volume_admitted: usize,
    pub maximum_volume_service: Option<f64>,
    pub proven_optimal: bool,
    pub bound: Option<f64>,
    pub relative_gap: Option<f64>,
    pub message: String,
}

fn policy_version() -> String {
    crate::team_policy::VERSION.to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkClassDemand {
    pub class: WorkClass,
    pub condition: String,
    pub resource_factor: f64,
    pub forecast_changes: usize,
    pub admitted_changes: usize,
    #[serde(default)]
    pub research_included: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolverReport {
    pub status: String,
    pub proven_optimal: bool,
    pub quality: f64,
    pub bound: Option<f64>,
    pub relative_gap: Option<f64>,
    pub nodes: usize,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledVisit {
    pub change_index: usize,
    pub class: WorkClass,
    pub seat: Seat,
    pub visit: usize,
    pub stage: usize,
    pub row_index: usize,
    pub provider_id: String,
    pub account_ordinal: usize,
    pub start_hours: f64,
    pub end_hours: f64,
    pub condition: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleAllocation {
    pub seat: Seat,
    pub allocated_hours: f64,
    pub quality_utility: f64,
    pub rules: Vec<DispatchRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchRule {
    pub lower_effort: Option<AdaptiveRoute>,
    pub within_provider_alternatives: Vec<AdaptiveRoute>,
    pub surplus_alternatives: Vec<AdaptiveRoute>,
    #[serde(default)]
    pub research_candidates: Vec<ResearchCandidate>,
    pub class: WorkClass,
    #[serde(default)]
    pub policy_id: String,
    #[serde(default)]
    pub planned_jobs: usize,
    #[serde(default)]
    pub recommendation_only: bool,
    #[serde(default)]
    pub account_claim: String,
    #[serde(default)]
    pub fallback_policy: String,
    #[serde(default)]
    pub calibration: String,
    pub condition: String,
    pub row_index: Option<usize>,
    pub attempt_limit: usize,
    pub expected_visits: f64,
    pub reserved_visits: usize,
    pub expected_completions: f64,
    pub competence: Option<f64>,
    pub utility: f64,
    pub nominal_usage: f64,
    pub nominal_hours: f64,
    pub reserved_usage: f64,
    pub reserved_hours: f64,
    pub per_call_expected_usage: f64,
    pub per_call_expected_hours: f64,
    pub per_call_reserved_usage: f64,
    pub per_call_reserved_hours: f64,
    pub provider_id: Option<String>,
    pub plan_id: Option<String>,
    pub fable: bool,
    pub failure_action: String,
    pub message: String,
    #[serde(default)]
    pub evidence_level: String,
    #[serde(default)]
    pub capability_sensitivity: Option<CapabilityRange>,
    #[serde(default)]
    pub repair_activation_proxy: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchCandidate {
    pub row_index: usize,
    pub binding_id: String,
    pub native_harness: String,
    pub provider_id: String,
    pub plan_id: String,
    pub primary: bool,
    pub score: f64,
    pub accuracy: f64,
    pub accuracy_weight: f64,
    pub non_wrong: f64,
    pub non_wrong_weight: f64,
    pub conditional_hallucination: f64,
    pub lcr: f64,
    pub lcr_weight: f64,
    pub hle: Option<f64>,
    pub hle_weight: f64,
    pub gpqa_diagnostic: Option<f64>,
    pub gdp_pdf_diagnostic: Option<f64>,
    pub expected_usd: Option<f64>,
    pub decode_hours: Option<f64>,
    pub source: String,
    pub eligibility: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchTierPick {
    pub tier: crate::types::Tier,
    pub primary: Option<ResearchCandidate>,
    pub alternatives: Vec<ResearchCandidate>,
    pub scope: String,
}

struct ResearchAssessment {
    score: f64,
    accuracy: f64,
    non_wrong: f64,
    conditional_hallucination: f64,
    lcr: f64,
    hle: Option<f64>,
    expected_usd: Option<f64>,
    decode_hours: Option<f64>,
}

fn assess_research(row: &Row, weights: [f64; 4]) -> Option<ResearchAssessment> {
    let bounded =
        |value: Option<f64>| value.filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
    let accuracy = bounded(row.omniscience_accuracy)?;
    let conditional_hallucination = bounded(row.halluc)?;
    let non_wrong = 1.0 - conditional_hallucination * (1.0 - accuracy);
    let lcr = bounded(row.lcr)?;
    let hle = if weights[3] > 0.0 {
        Some(bounded(row.hle)?)
    } else {
        bounded(row.hle)
    };
    let direct_metric = |benchmark| {
        row.task_metrics
            .iter()
            .find(|metric| metric.benchmark == benchmark && metric.canonical_resources)
    };
    let omni_weight = weights[0] + weights[1];
    let resources = direct_metric(crate::types::Benchmark::Omniscience)
        .zip(direct_metric(crate::types::Benchmark::Lcr))
        .and_then(|(omni, lcr)| {
            if weights[3] > 0.0 {
                direct_metric(crate::types::Benchmark::Hle).map(|hle| (omni, lcr, Some(hle)))
            } else {
                Some((omni, lcr, None))
            }
        });
    Some(ResearchAssessment {
        score: engine::weighted_geometric(&[
            (accuracy, weights[0]),
            (non_wrong, weights[1]),
            (lcr, weights[2]),
            (hle.unwrap_or(0.0), weights[3]),
        ])?,
        accuracy,
        non_wrong,
        conditional_hallucination,
        lcr,
        hle,
        expected_usd: resources.map(|(omni, lcr, hle)| {
            omni_weight * omni.usd
                + weights[2] * lcr.usd
                + weights[3] * hle.map_or(0.0, |metric| metric.usd)
        }),
        decode_hours: resources.map(|(omni, lcr, hle)| {
            (omni_weight * omni.seconds
                + weights[2] * lcr.seconds
                + weights[3] * hle.map_or(0.0, |metric| metric.seconds))
                / 3600.0
        }),
    })
}

fn research_sensitivity(candidate: &ResearchCandidate, span: f64) -> CapabilityRange {
    let weighted = |multiplier: f64| {
        engine::weighted_geometric(&[
            (
                (candidate.accuracy * multiplier).clamp(0.0, 1.0),
                candidate.accuracy_weight,
            ),
            (
                (candidate.non_wrong * multiplier).clamp(0.0, 1.0),
                candidate.non_wrong_weight,
            ),
            (
                (candidate.lcr * multiplier).clamp(0.0, 1.0),
                candidate.lcr_weight,
            ),
            (
                (candidate.hle.unwrap_or(0.0) * multiplier).clamp(0.0, 1.0),
                candidate.hle_weight,
            ),
        ])
        .unwrap_or(candidate.score)
    };
    CapabilityRange {
        nominal: candidate.score,
        lower: weighted(1.0 - span),
        upper: weighted(1.0 + span),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdaptiveRoute {
    pub row_index: usize,
    pub binding_id: String,
    pub provider_id: String,
    pub plan_id: String,
    pub attempt_limit: usize,
    pub competence: f64,
    pub quality: f64,
    pub per_call_reserved_usage: f64,
    pub per_call_reserved_hours: f64,
    pub per_call_expected_usage: f64,
    pub per_call_expected_hours: f64,
    pub regret: f64,
    pub opportunity_cost: f64,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolUsage {
    pub provider_id: String,
    pub subscription_count: usize,
    pub per_account_weekly_capacity: f64,
    #[serde(default)]
    pub per_account_fable_capacity: f64,
    #[serde(default)]
    pub windows: Vec<crate::types::CapacityWindow>,
    #[serde(default)]
    pub rate_windows: Vec<WindowRateUsage>,
    #[serde(default)]
    pub rate_infeasible: bool,
    pub provider_name: String,
    pub plan_id: String,
    pub plan_name: String,
    pub monthly_price: f64,
    pub price_is_estimate: bool,
    pub weekly_capacity: f64,
    pub nominal_usage: f64,
    pub stress_usage: f64,
    pub remaining_capacity: f64,
    pub utilization_pct: f64,
    pub available_hours: f64,
    pub scheduled_hours: f64,
    pub unused_hours: f64,
    pub reserved_usage: f64,
    pub reserved_hours: f64,
    pub remaining_reserved_capacity: f64,
    pub remaining_reserved_hours: f64,
    pub unit: String,
    pub source_url: String,
    pub allowance_basis: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolLimit {
    pub provider_id: String,
    pub name: String,
    pub weekly_capacity: f64,
    pub nominal_usage: f64,
    pub stress_usage: f64,
    pub reserved_usage: f64,
    pub remaining_capacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowRateUsage {
    pub window_id: String,
    pub unit: CapacityUnit,
    pub ceiling_per_hour: Option<f64>,
    pub committed_per_hour: Option<f64>,
    pub known_committed_per_hour: f64,
    pub binding: bool,
    pub status: String,
}

impl PoolUsage {
    fn rate_fits(
        &self,
        demand: CapacityDemand,
        duration_hours: f64,
        fable: bool,
        parallel: usize,
    ) -> bool {
        self.windows.iter().all(|window| {
            if !window.applies_to(&self.windows, fable) {
                return true;
            }
            let Ok(ceiling) = window.rate_ceiling(&self.windows) else {
                return true;
            };
            let Some(amount) = demand.amount(window.unit) else {
                return true;
            };
            parallel as f64 * amount / duration_hours <= ceiling
        })
    }
}

fn policy_rate(
    policy: &Policy,
    unit: CapacityUnit,
    conductor: bool,
    settings: &Settings,
) -> Option<f64> {
    if conductor {
        let demand = CapacityDemand {
            api_equivalent_usd: settings.orchestrator_headroom_usd,
            ..CapacityDemand::default()
        };
        demand
            .amount(unit)
            .map(|amount| amount / settings.agent_hours)
    } else {
        policy.cycle.rate(unit)
    }
}

fn refresh_rate_usage(portfolio: &mut Portfolio, rows: &[Row], settings: &Settings) {
    for pool in &mut portfolio.pools {
        let mut usages = Vec::new();
        for window in &pool.windows {
            let ceiling = window.rate_ceiling(&pool.windows);
            let mut unknown = false;
            let mut committed = Vec::new();
            for class in WorkClass::ALL {
                let mut rate = 0.0;
                for role in &portfolio.roles {
                    let rules = role.rules.iter().filter(|rule| {
                        rule.class == class
                            && !rule.recommendation_only
                            && rule.planned_jobs > 0
                            && rule.provider_id.as_deref() == Some(&pool.provider_id)
                    });
                    for rule in rules {
                        if window.applies_to(&pool.windows, rule.fable) {
                            if window.unit == CapacityUnit::ApiEquivalentUsd
                                && rule.per_call_reserved_hours > 0.0
                            {
                                rate += rule.per_call_reserved_usage / rule.per_call_reserved_hours;
                            } else {
                                unknown = true;
                            }
                        }
                    }
                }
                if let Some(conductor) = portfolio
                    .conductor
                    .as_ref()
                    .filter(|conductor| conductor.provider_id == pool.provider_id)
                    && window.applies_to(
                        &pool.windows,
                        subscriptions::is_fable(&rows[conductor.row_index]),
                    )
                {
                    if let Some(amount) = settings
                        .orchestrator_headroom_usd
                        .filter(|_| window.unit == CapacityUnit::ApiEquivalentUsd)
                    {
                        rate += amount / settings.agent_hours;
                    } else {
                        unknown = true;
                    }
                }
                committed.push(settings.orchestrators as f64 * rate);
            }
            let rate = committed.into_iter().fold(0.0, f64::max);
            let binding = ceiling.as_ref().is_ok_and(|ceiling| {
                !pool.windows.iter().any(|other| {
                    other.unit == window.unit
                        && other.applies_to(&pool.windows, false)
                            == window.applies_to(&pool.windows, false)
                        && other
                            .rate_ceiling(&pool.windows)
                            .is_ok_and(|rate| rate < *ceiling)
                })
            });
            let status = match ceiling {
                Err(reason) => reason.to_owned(),
                Ok(ceiling) if rate > ceiling => "rate-infeasible".to_owned(),
                Ok(_) if window.unit != CapacityUnit::ApiEquivalentUsd => format!(
                    "{} demand unknown; measured native counts per seat must be supplied; known same-unit demand is enforced",
                    window.unit.label(),
                ),
                Ok(_) if unknown => {
                    "known committed rate fits; ongoing Orchestrator demand unknown".to_owned()
                }
                Ok(_) => "committed rate fits".to_owned(),
            };
            usages.push(WindowRateUsage {
                window_id: window.id.clone(),
                unit: window.unit,
                ceiling_per_hour: ceiling.ok(),
                committed_per_hour: (!unknown).then_some(rate),
                known_committed_per_hour: rate,
                binding,
                status,
            });
        }
        pool.rate_windows = usages;
    }
}
fn reference_path_hours(candidates: &[Vec<Policy>], settings: &Settings) -> Option<f64> {
    WorkClass::ALL
        .into_iter()
        .zip(CLASS_MIX)
        .map(|(class, weight)| {
            let research = settings
                .research_classes
                .get(class.name())
                .copied()
                .unwrap_or(false);
            let stages = workflow_stages(class, research);
            let mut finishes = vec![0.0; stages.len()];
            for stage in &stages {
                let role = Seat::ALL.iter().position(|seat| *seat == stage.seat)?;
                let duration = candidates[role]
                    .iter()
                    .filter(|policy| {
                        policy
                            .research_class
                            .is_none_or(|policy_class| policy_class == class)
                    })
                    .map(|policy| policy.cycle.reserved_seconds / 3600.0 * class.resource_factor())
                    .reduce(f64::min)?;
                let dependency_finish = stage
                    .dependencies
                    .iter()
                    .map(|dependency| finishes[*dependency])
                    .fold(0.0, f64::max);
                finishes[stage.id] = dependency_finish + duration;
            }
            Some(weight * finishes.into_iter().reduce(f64::max)?)
        })
        .sum()
}

#[derive(Clone)]
struct Policy {
    row_index: usize,
    native_harness: String,
    pool: usize,
    competence: f64,
    utility: f64,
    attempt_limit: usize,
    cycle: DispatchCycle,
    fable: bool,
    research_class: Option<WorkClass>,
    lower_utility: f64,
    upper_utility: f64,
    evidence_level: String,
}

struct Group {
    role: usize,
    class: Option<WorkClass>,
    policies: Vec<Policy>,
}

#[derive(Default)]
struct AccountCalendar {
    intervals: Vec<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConductorAllocation {
    pub row_index: usize,
    pub binding_id: String,
    pub native_harness: String,
    pub provider_id: String,
    pub plan_id: String,
    pub attempt_limit: usize,
    pub competence: f64,
    pub utility: f64,
    #[serde(default = "persistent_conductor")]
    pub persistent: bool,
    #[serde(default)]
    pub evidence_profile: String,
    #[serde(default)]
    pub evidence_level: String,
    #[serde(default)]
    pub capability_sensitivity: Option<CapabilityRange>,
    #[serde(default)]
    pub usage_forecast: Option<ConductorUsageForecast>,
    #[serde(default)]
    pub headroom: ConductorHeadroom,
    pub per_call_expected_usage: f64,
    pub per_call_expected_hours: f64,
    pub per_call_reserved_usage: f64,
    pub per_call_reserved_hours: f64,
    pub cost_basis: String,
    pub time_basis: String,
    pub classes: Vec<ConductorClassDemand>,
}

fn persistent_conductor() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConductorUsageForecast {
    pub visits: usize,
    pub usage: f64,
    pub hours: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConductorHeadroom {
    pub usd: Option<f64>,
    pub hours: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConductorTradeoff {
    pub competence_floor: f64,
    pub row_index: usize,
    pub binding_id: String,
    pub admitted_changes: usize,
    pub quality: f64,
    pub bound: Option<f64>,
    pub relative_gap: Option<f64>,
    pub proven: bool,
    pub pool_claims: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConductorClassDemand {
    pub class: WorkClass,
    pub changes: usize,
    pub resource_factor: f64,
    pub visits: usize,
}

#[derive(Clone)]
struct Schedule {
    visits: Vec<ScheduledVisit>,
    makespan: f64,
}

fn search_service(search: &Search) -> Option<f64> {
    let solution = search.state.incumbent()?;
    Some(search.workload * ((solution.quality + search.log_shift) / search.workload).exp())
}

fn search_service_bound(search: &Search) -> f64 {
    if search.state.proven() && !search.state.has_solution() {
        return 0.0;
    }
    search.state.bound().map_or(search.workload, |bound| {
        search.workload * ((bound + search.log_shift) / search.workload).exp()
    })
}

struct Search {
    groups: Vec<Group>,
    problem: Problem,
    state: solver::SolverState<Schedule>,
    count: usize,
    workload: f64,
    log_shift: f64,
}

fn failure_action(seat: Seat) -> &'static str {
    match seat {
        Seat::Implementer => {
            "On blocked implementation after the bounded attempts, run the one funded Debugger visit, then the two rechecks."
        }
        Seat::Debugger => {
            "If the bounded repair fails, DEFER to the user; do not select another repair model."
        }
        Seat::Reviewer | Seat::Sanity => {
            "Initial rejection triggers the one funded repair; rejection after repair means DEFER to the user."
        }
        _ => {
            "If the bounded visit fails, DEFER to the user; do not introduce an unreserved alternative."
        }
    }
}

fn seat_id(seat: Seat) -> &'static str {
    match seat {
        Seat::Implementer => "implementer",
        Seat::Debugger => "debugger",
        Seat::Reviewer => "reviewer",
        Seat::Orchestrator => "orchestrator",
        Seat::Sanity => "sanity",
        Seat::Comprehension => "comprehension",
        Seat::NetResearch => "net-research",
    }
}

fn evidence_level_name(level: engine::RoleEvidenceLevel) -> String {
    match level {
        engine::RoleEvidenceLevel::ExactHarness => "exact_harness",
        engine::RoleEvidenceLevel::ModelLevel => "model_level",
        engine::RoleEvidenceLevel::CrossHarnessProxy => "cross_harness_proxy",
        engine::RoleEvidenceLevel::Unknown => "unknown",
    }
    .to_owned()
}

fn role_evidence_level(row: &Row, seat: Seat) -> String {
    engine::role_evidence(row, seat)
        .map(|components| evidence_level_name(engine::weakest_evidence_level(&components)))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn research_evidence_level(row: &Row, class: WorkClass) -> String {
    let families: &[&str] = if matches!(class, WorkClass::Complex | WorkClass::Extensive) {
        &["omniscience", "lcr", "hle"]
    } else {
        &["omniscience", "lcr"]
    };
    let level = families
        .iter()
        .map(|family| {
            row.benchmark_evidence
                .iter()
                .find(|projection| projection.family == *family)
                .map_or(
                    engine::RoleEvidenceLevel::Unknown,
                    |projection| match projection.transfer {
                        crate::evidence::TransferKind::ExactHarness => {
                            engine::RoleEvidenceLevel::ExactHarness
                        }
                        crate::evidence::TransferKind::ModelLevel => {
                            engine::RoleEvidenceLevel::ModelLevel
                        }
                        crate::evidence::TransferKind::CrossHarnessProxy => {
                            engine::RoleEvidenceLevel::CrossHarnessProxy
                        }
                    },
                )
        })
        .min_by_key(|level| match level {
            engine::RoleEvidenceLevel::Unknown => 0,
            engine::RoleEvidenceLevel::CrossHarnessProxy => 1,
            engine::RoleEvidenceLevel::ModelLevel => 2,
            engine::RoleEvidenceLevel::ExactHarness => 3,
        })
        .unwrap_or(engine::RoleEvidenceLevel::Unknown);
    evidence_level_name(level)
}

fn sequence(total: usize) -> Vec<WorkClass> {
    let weights = CLASS_MIX;
    let mut counts = weights.map(|weight| (total as f64 * weight).floor() as usize);
    let mut remainder: Vec<_> = (0..4).collect();
    remainder.sort_by(|left, right| {
        (total as f64 * weights[*right] - counts[*right] as f64)
            .total_cmp(&(total as f64 * weights[*left] - counts[*left] as f64))
            .then_with(|| left.cmp(right))
    });
    for index in remainder
        .into_iter()
        .take(total - counts.iter().sum::<usize>())
    {
        counts[index] += 1;
    }
    let mut placed = [0; 4];
    (0..total)
        .map(|step| {
            let class = (0..4)
                .filter(|index| placed[*index] < counts[*index])
                .max_by(|left, right| {
                    ((step + 1) as f64 * weights[*left] - placed[*left] as f64)
                        .total_cmp(&((step + 1) as f64 * weights[*right] - placed[*right] as f64))
                        .then_with(|| right.cmp(left))
                })
                .unwrap_or(0);
            placed[class] += 1;
            WorkClass::ALL[class]
        })
        .collect()
}

fn prune(mut policies: Vec<Policy>, rows: &[Row], preserve_competence: bool) -> Vec<Policy> {
    policies.sort_by(|left, right| {
        (rows[left.row_index].harness != "model")
            .cmp(&(rows[right.row_index].harness != "model"))
            .then_with(|| {
                rows[left.row_index]
                    .display_name()
                    .cmp(&rows[right.row_index].display_name())
            })
            .then_with(|| {
                rows[left.row_index]
                    .harness
                    .cmp(&rows[right.row_index].harness)
            })
            .then_with(|| left.attempt_limit.cmp(&right.attempt_limit))
    });
    let retain: Vec<_> = policies
        .iter()
        .enumerate()
        .map(|(index, policy)| {
            if policy.research_class.is_some() {
                return true;
            }
            !policies.iter().enumerate().any(|(other_index, other)| {
                other_index != index
                    && other.pool == policy.pool
                    && (!preserve_competence || other.competence == policy.competence)
                    && (!other.fable || policy.fable)
                    && other.utility >= policy.utility
                    && other.cycle.reserved_usage <= policy.cycle.reserved_usage
                    && other.cycle.reserved_seconds == policy.cycle.reserved_seconds
                    && other.cycle.nominal_usage <= policy.cycle.nominal_usage
                    && other.cycle.nominal_seconds <= policy.cycle.nominal_seconds
                    && (other.utility > policy.utility
                        || other.cycle.reserved_usage < policy.cycle.reserved_usage
                        || other.cycle.nominal_usage < policy.cycle.nominal_usage
                        || other.cycle.nominal_seconds < policy.cycle.nominal_seconds
                        || other_index < index)
            })
        })
        .collect();
    let mut policies: Vec<_> = policies
        .into_iter()
        .zip(retain)
        .filter_map(|(policy, keep)| keep.then_some(policy))
        .collect();
    policies.sort_by(|left, right| {
        (right.utility)
            .total_cmp(&(left.utility))
            .then_with(|| {
                left.cycle
                    .nominal_seconds
                    .total_cmp(&right.cycle.nominal_seconds)
            })
            .then_with(|| {
                left.cycle
                    .nominal_usage
                    .total_cmp(&right.cycle.nominal_usage)
            })
            .then_with(|| left.row_index.cmp(&right.row_index))
            .then_with(|| left.attempt_limit.cmp(&right.attempt_limit))
    });
    policies
}

fn schedule(
    groups: &[Group],
    choices: &[usize],
    sequence: &[WorkClass],
    pools: &[PoolUsage],
    settings: &Settings,
) -> Option<Schedule> {
    for (pool_index, pool) in pools.iter().enumerate() {
        for window in &pool.windows {
            let Ok(ceiling) = window.rate_ceiling(&pool.windows) else {
                continue;
            };
            for class in WorkClass::ALL {
                let rate: f64 = groups
                    .iter()
                    .zip(choices)
                    .filter_map(|(group, choice)| {
                        let policy = &group.policies[*choice];
                        if policy.pool != pool_index
                            || group.class.is_some_and(|selected| selected != class)
                            || !window.applies_to(&pool.windows, policy.fable)
                        {
                            return None;
                        }
                        policy_rate(policy, window.unit, group.class.is_none(), settings)
                    })
                    .sum();
                if settings.orchestrators as f64 * rate > ceiling {
                    return None;
                }
            }
        }
    }
    let hours = settings.agent_hours;
    let research_classes = &settings.research_classes;
    let orchestrator_headroom_hours = settings.orchestrator_headroom_hours;
    let mut lookup = [[None; 7]; 4];
    let mut orchestrator = None;
    for (index, group) in groups.iter().enumerate() {
        if let Some(class) = group.class {
            lookup[class.index()][group.role] = Some(index);
        } else {
            orchestrator = Some(index);
        }
    }
    let mut planned = Vec::new();
    for (change_index, class) in sequence.iter().copied().enumerate() {
        let research = research_classes.get(class.name()).copied().unwrap_or(false);
        let stages = workflow_stages(class, research);
        let base = planned.len();
        for stage in stages {
            let role = Seat::ALL.iter().position(|seat| *seat == stage.seat)?;
            let group = if stage.seat == Seat::Orchestrator {
                orchestrator?
            } else {
                lookup[class.index()][role]?
            };
            planned.push((
                change_index,
                class,
                base,
                stage,
                &groups[group].policies[choices[group]],
            ));
        }
    }
    let mut finishes = vec![0.0_f64; planned.len()];
    let mut calendars: Vec<Vec<AccountCalendar>> = (0..pools.len()).map(|_| Vec::new()).collect();
    if let Some(group) = orchestrator {
        let policy = &groups[group].policies[choices[group]];
        let held_hours = orchestrator_headroom_hours.unwrap_or(0.0);
        calendars[policy.pool].push(AccountCalendar {
            intervals: (held_hours > 0.0)
                .then_some((hours - held_hours, hours))
                .into_iter()
                .collect(),
        });
    }
    let mut visits = Vec::with_capacity(planned.len());
    let mut makespan = 0.0_f64;
    for (index, (change_index, class, base, stage, policy)) in planned.iter().enumerate() {
        let duration = policy.cycle.reserved_seconds * class.resource_factor() / 3600.0;
        let dependency_start = stage
            .dependencies
            .iter()
            .map(|dependency| finishes[base + dependency])
            .fold(0.0, f64::max);
        if calendars[policy.pool].is_empty() {
            calendars[policy.pool].push(AccountCalendar::default());
        }
        let (account_ordinal, start, finish) = (0..1)
            .filter_map(|account| {
                let mut start = dependency_start;
                if let Some(calendar) = calendars[policy.pool].get(account) {
                    for (occupied_start, occupied_end) in &calendar.intervals {
                        if start + duration <= *occupied_start {
                            break;
                        }
                        if start < *occupied_end {
                            start = *occupied_end;
                        }
                    }
                }
                let finish = start + duration;
                (finish <= hours + 1e-8).then_some((account, start, finish))
            })
            .min_by(|left, right| {
                left.2
                    .total_cmp(&right.2)
                    .then_with(|| left.0.cmp(&right.0))
            })?;
        if account_ordinal == calendars[policy.pool].len() {
            calendars[policy.pool].push(AccountCalendar::default());
        }
        finishes[index] = finish;
        makespan = makespan.max(finish);
        let calendar = &mut calendars[policy.pool][account_ordinal];
        let position = calendar
            .intervals
            .partition_point(|(occupied_start, _)| *occupied_start <= start);
        calendar.intervals.insert(position, (start, finish));
        visits.push(ScheduledVisit {
            change_index: *change_index,
            class: *class,
            seat: stage.seat,
            visit: stage.visit,
            stage: stage.id,
            row_index: policy.row_index,
            provider_id: pools[policy.pool].provider_id.clone(),
            account_ordinal: account_ordinal + 1,
            start_hours: start,
            end_hours: finish,
            condition: stage.condition.to_owned(),
        });
    }
    Some(Schedule { visits, makespan })
}

fn search(
    count: usize,
    sequence: &[WorkClass],
    candidates: &[Vec<Policy>],
    pools: &[PoolUsage],
    settings: &Settings,
    limit: usize,
    stressed_incumbents: Option<&std::collections::BTreeSet<String>>,
) -> Search {
    let hours = settings.agent_hours;
    let research_classes = &settings.research_classes;
    let monotonic_class_competence = settings.monotonic_class_competence;
    let mut counts = [0_usize; 4];
    for class in &sequence[..count] {
        counts[class.index()] += 1;
    }
    let mut groups = Vec::new();
    let orchestrator_role = Seat::ALL
        .iter()
        .position(|seat| *seat == Seat::Orchestrator)
        .unwrap_or(0);
    groups.push(Group {
        role: orchestrator_role,
        class: None,
        policies: candidates[orchestrator_role]
            .iter()
            .filter(|policy| {
                stressed_incumbents.map_or(policy.utility, |incumbents| {
                    if incumbents.contains(&stress_identity(orchestrator_role, policy)) {
                        policy.lower_utility
                    } else {
                        policy.upper_utility
                    }
                }) > 0.0
            })
            .cloned()
            .collect(),
    });
    for class in WorkClass::ALL {
        if counts[class.index()] > 0 {
            for (role, policies) in candidates.iter().enumerate() {
                if role == orchestrator_role
                    || (Seat::ALL[role] == Seat::NetResearch
                        && !research_classes.get(class.name()).copied().unwrap_or(false))
                    || reserved_visits(
                        class,
                        Seat::ALL[role],
                        research_classes.get(class.name()).copied().unwrap_or(false),
                    ) == 0
                {
                    continue;
                }
                groups.push(Group {
                    role,
                    class: Some(class),
                    policies: policies
                        .iter()
                        .filter(|policy| {
                            (Seat::ALL[role] != Seat::NetResearch
                                || policy.research_class == Some(class))
                                && stressed_incumbents.map_or(policy.utility, |incumbents| {
                                    if incumbents.contains(&stress_identity(role, policy)) {
                                        policy.lower_utility
                                    } else {
                                        policy.upper_utility
                                    }
                                }) > 0.0
                        })
                        .cloned()
                        .collect(),
                });
            }
        }
    }
    let mut capacities: Vec<_> = pools.iter().map(|pool| pool.available_hours).collect();
    let mut rate_resources = Vec::new();
    for (pool_index, pool) in pools.iter().enumerate() {
        for (window_index, window) in pool.windows.iter().enumerate() {
            if let Ok(ceiling) = window.rate_ceiling(&pool.windows) {
                for class in WorkClass::ALL {
                    if counts[class.index()] > 0 {
                        rate_resources.push((pool_index, window_index, class, capacities.len()));
                        capacities.push(ceiling);
                    }
                }
            }
        }
    }
    let path_resource = capacities.len();
    capacities.extend([hours, hours]);
    let mut competence_links = Vec::new();
    if monotonic_class_competence {
        for role in 0..Seat::ALL.len() {
            if matches!(Seat::ALL[role], Seat::Orchestrator | Seat::NetResearch) {
                continue;
            }
            let present: Vec<_> = WorkClass::ALL
                .into_iter()
                .filter_map(|class| {
                    groups
                        .iter()
                        .position(|group| group.role == role && group.class == Some(class))
                })
                .collect();
            competence_links.extend(present.windows(2).map(|pair| (pair[0], pair[1])));
        }
        capacities.extend(std::iter::repeat_n(1.0, competence_links.len()));
    }
    let diversity_resource = capacities.len();
    let mut diversity_keys: Vec<(WorkClass, usize)> = Vec::new();
    for class in WorkClass::ALL {
        if class == WorkClass::Focused || counts[class.index()] == 0 {
            continue;
        }
        let rows: BTreeSet<usize> = groups
            .iter()
            .filter(|group| {
                group.class == Some(class)
                    && matches!(
                        Seat::ALL[group.role],
                        Seat::Implementer | Seat::Reviewer | Seat::Sanity
                    )
            })
            .flat_map(|group| group.policies.iter().map(|policy| policy.row_index))
            .collect();
        for row_index in rows {
            diversity_keys.push((class, row_index));
            capacities.push(1.0);
        }
    }
    let choices = groups
        .iter()
        .enumerate()
        .map(|(group_index, group)| {
            let demands: Vec<_> = match group.class {
                Some(class) => vec![(counts[class.index()] as f64, class.resource_factor())],
                None => WorkClass::ALL
                    .into_iter()
                    .map(|class| (counts[class.index()] as f64, class.resource_factor()))
                    .collect(),
            };
            let quality_factor: f64 = if let Some(class) = group.class {
                let workers = groups
                    .iter()
                    .filter(|candidate| candidate.class == Some(class))
                    .count() as f64;
                counts[class.index()] as f64
                    * class.resource_factor()
                    * (1.0 - ORCHESTRATOR_WELFARE)
                    / workers
            } else {
                demands
                    .iter()
                    .map(|(changes, factor)| changes * factor * ORCHESTRATOR_WELFARE)
                    .sum()
            };
            let reserved_factor: f64 = demands
                .iter()
                .map(|(changes, factor)| {
                    let class = group.class.unwrap_or(WorkClass::Focused);
                    changes
                        * factor
                        * if group.class.is_none() {
                            0.0
                        } else {
                            reserved_visits(
                                class,
                                Seat::ALL[group.role],
                                research_classes.get(class.name()).copied().unwrap_or(false),
                            ) as f64
                        }
                })
                .sum();
            let minimum_log_utility = group
                .policies
                .iter()
                .map(|policy| {
                    stressed_incumbents.map_or(policy.utility, |incumbents| {
                        if incumbents.contains(&stress_identity(group.role, policy)) {
                            policy.lower_utility
                        } else {
                            policy.upper_utility
                        }
                    })
                })
                .map(f64::ln)
                .reduce(f64::min)
                .unwrap_or(0.0);
            group
                .policies
                .iter()
                .map(|policy| {
                    let utility = stressed_incumbents.map_or(policy.utility, |incumbents| {
                        if incumbents.contains(&stress_identity(group.role, policy)) {
                            policy.lower_utility
                        } else {
                            policy.upper_utility
                        }
                    });
                    let mut resources = vec![0.0; capacities.len()];
                    resources[policy.pool] = if group.class.is_none() {
                        settings.orchestrator_headroom_hours.unwrap_or(0.0)
                    } else {
                        reserved_factor * policy.cycle.reserved_seconds / 3600.0
                    };
                    for (pool_index, window_index, class, resource) in &rate_resources {
                        if *pool_index != policy.pool
                            || group.class.is_some_and(|selected| selected != *class)
                        {
                            continue;
                        }
                        let pool = &pools[*pool_index];
                        let window = &pool.windows[*window_index];
                        if window.applies_to(&pool.windows, policy.fable)
                            && let Some(rate) =
                                policy_rate(policy, window.unit, group.class.is_none(), settings)
                        {
                            resources[*resource] = settings.orchestrators as f64 * rate;
                        }
                    }
                    for (link_index, (lower, upper)) in competence_links.iter().enumerate() {
                        let resource = path_resource + 2 + link_index;
                        if group_index == *lower {
                            resources[resource] = policy.competence;
                        } else if group_index == *upper {
                            resources[resource] = 1.0 - policy.competence;
                        }
                    }
                    for (class, changes) in WorkClass::ALL.into_iter().zip(counts) {
                        if group.class.is_some_and(|selected| selected != class) || changes == 0 {
                            continue;
                        }
                        let path = path_resource;
                        let factor = class.resource_factor();
                        let seat = Seat::ALL[group.role];
                        let research = research_classes.get(class.name()).copied().unwrap_or(false);
                        let visits = reserved_visits(class, seat, research) as f64;
                        let jobs = changes as f64;
                        resources[path] += jobs
                            * if seat == Seat::Sanity { 0.0 } else { visits }
                            * factor
                            * policy.cycle.reserved_seconds
                            / 3600.0;
                        resources[path + 1] += jobs
                            * if seat == Seat::Reviewer { 0.0 } else { visits }
                            * factor
                            * policy.cycle.reserved_seconds
                            / 3600.0;
                    }
                    for (offset, (class, row_index)) in diversity_keys.iter().enumerate() {
                        if group.class == Some(*class)
                            && matches!(
                                Seat::ALL[group.role],
                                Seat::Implementer | Seat::Reviewer | Seat::Sanity
                            )
                            && policy.row_index == *row_index
                        {
                            resources[diversity_resource + offset] = 1.0;
                        }
                    }
                    Choice {
                        quality: quality_factor * (utility.ln() - minimum_log_utility),
                        nominal_time: if group.class.is_none() {
                            0.0
                        } else {
                            quality_factor * policy.cycle.nominal_seconds / 3600.0
                        },
                        nominal_cost: if group.class.is_none() {
                            0.0
                        } else {
                            quality_factor * policy.cycle.nominal_usage
                        },
                        resources,
                    }
                })
                .collect()
        })
        .collect();
    let problem = Problem {
        groups: choices,
        capacities,
    };
    let mut state = solver::begin(&problem, limit, |choices| {
        schedule(&groups, choices, &sequence[..count], pools, settings)
    });
    solver::advance(&problem, &mut state, limit, true, |choices| {
        schedule(&groups, choices, &sequence[..count], pools, settings)
    });
    let workload = counts
        .into_iter()
        .zip(WorkClass::ALL)
        .map(|(changes, class)| changes as f64 * class.resource_factor())
        .sum();
    let log_shift = groups
        .iter()
        .map(|group| {
            let coefficient = if let Some(class) = group.class {
                let workers = groups
                    .iter()
                    .filter(|candidate| candidate.class == Some(class))
                    .count()
                    .max(1) as f64;
                counts[class.index()] as f64
                    * class.resource_factor()
                    * (1.0 - ORCHESTRATOR_WELFARE)
                    / workers
            } else {
                WorkClass::ALL
                    .into_iter()
                    .map(|class| {
                        counts[class.index()] as f64
                            * class.resource_factor()
                            * ORCHESTRATOR_WELFARE
                    })
                    .sum()
            };
            let minimum = group
                .policies
                .iter()
                .map(|policy| {
                    stressed_incumbents.map_or(policy.utility, |incumbents| {
                        if incumbents.contains(&stress_identity(group.role, policy)) {
                            policy.lower_utility
                        } else {
                            policy.upper_utility
                        }
                    })
                })
                .map(f64::ln)
                .reduce(f64::min)
                .unwrap_or(0.0);
            coefficient * minimum
        })
        .sum();
    Search {
        groups,
        problem,
        state,
        count,
        workload,
        log_shift,
    }
}

fn policy_identity(role: usize, policy: &Policy) -> String {
    format!(
        "{role}:{}:{}:{}:{}:{:?}",
        policy.row_index,
        policy.attempt_limit,
        policy.native_harness,
        policy.pool,
        policy.research_class
    )
}

fn stress_identity(role: usize, policy: &Policy) -> String {
    format!(
        "{role}:{}:{}:{}:{:?}",
        policy.row_index, policy.native_harness, policy.pool, policy.research_class
    )
}

fn conductor_headroom_fits(pool: &PoolUsage, fable: bool, settings: &Settings) -> bool {
    let usage = settings.orchestrator_headroom_usd.unwrap_or(0.0);
    let hours = settings.orchestrator_headroom_hours.unwrap_or(0.0);
    pool.rate_fits(
        CapacityDemand {
            api_equivalent_usd: Some(usage),
            ..CapacityDemand::default()
        },
        settings.agent_hours,
        fable,
        settings.orchestrators,
    ) && hours <= settings.agent_hours
}

fn scenario_better_or_equal<T>(
    candidate: &solver::Solution<T>,
    baseline_quality: f64,
    baseline_time: f64,
    baseline_cost: f64,
) -> bool {
    candidate.quality > baseline_quality + 1e-8
        || ((candidate.quality - baseline_quality).abs() <= 1e-8
            && (candidate.nominal_time < baseline_time - 1e-8
                || ((candidate.nominal_time - baseline_time).abs() <= 1e-8
                    && candidate.nominal_cost <= baseline_cost + 1e-8)))
}

fn capability_scenario(
    count: usize,
    sequence: &[WorkClass],
    all_candidates: &[Vec<Policy>],
    pools: &[PoolUsage],
    settings: &Settings,
    nominal_groups: &[Group],
    nominal_choices: &[usize],
) -> (CapabilityScenarioReport, Vec<CloseChoice>) {
    let incumbents: std::collections::BTreeSet<_> = nominal_groups
        .iter()
        .zip(nominal_choices)
        .map(|(group, choice)| stress_identity(group.role, &group.policies[*choice]))
        .collect();
    let mut result = search(
        count,
        sequence,
        all_candidates,
        pools,
        settings,
        256,
        Some(&incumbents),
    );
    let used = result.state.nodes();
    solver::advance(
        &result.problem,
        &mut result.state,
        256_usize.saturating_sub(used),
        false,
        |choices| schedule(&result.groups, choices, &sequence[..count], pools, settings),
    );
    let baseline_choices: Option<Vec<_>> = result
        .groups
        .iter()
        .map(|group| {
            let nominal_group = nominal_groups
                .iter()
                .position(|nominal| nominal.role == group.role && nominal.class == group.class)?;
            let nominal = &nominal_groups[nominal_group].policies[nominal_choices[nominal_group]];
            group.policies.iter().position(|policy| {
                policy_identity(group.role, policy) == policy_identity(group.role, nominal)
            })
        })
        .collect();
    let workload = result.workload;
    let log_shift = result.log_shift;
    let Some((baseline_quality, baseline_time, baseline_cost)) =
        baseline_choices.as_ref().map(|choices| {
            result.problem.groups.iter().zip(choices).fold(
                (0.0, 0.0, 0.0),
                |totals, (group, choice)| {
                    let selected = &group[*choice];
                    (
                        totals.0 + selected.quality,
                        totals.1 + selected.nominal_time,
                        totals.2 + selected.nominal_cost,
                    )
                },
            )
        })
    else {
        return (
                CapabilityScenarioReport {
                    status: "unresolved".to_owned(),
                    proven: false,
                    quality: 0.0,
                    baseline_quality: None,
                    bound: None,
                    relative_gap: None,
                    message: "The bounded sensitivity scenario could not map the complete nominal team into its eligible candidate set; stability is unresolved.".to_owned(),
                    assignments: Vec::new(),
                },
                Vec::new(),
            );
    };
    let baseline_service = workload * ((baseline_quality + log_shift) / workload).exp();
    let outcome = solver::finish(result.state);
    let bound = outcome
        .bound
        .map(|value| workload * ((value + log_shift) / workload).exp());
    let proven = outcome.proven;
    let Some(solution) = outcome.solution.filter(|solution| {
        scenario_better_or_equal(solution, baseline_quality, baseline_time, baseline_cost)
    }) else {
        return (
            CapabilityScenarioReport {
                status: "unresolved".to_owned(),
                proven: false,
                quality: 0.0,
                baseline_quality: Some(baseline_service),
                bound,
                relative_gap: None,
                message: "The bounded role-specific sensitivity search did not find a complete plan that matches the nominal team's stressed objective; this does not establish stability.".to_owned(),
                assignments: Vec::new(),
            },
            Vec::new(),
        );
    };
    let service = workload * ((solution.quality + log_shift) / workload).exp();
    let relative_gap = bound.map(|value| (value / service.max(f64::MIN_POSITIVE) - 1.0).max(0.0));
    let mut assignments = Vec::new();
    let mut changes = Vec::new();
    for (group, choice) in result.groups.iter().zip(&solution.choices) {
        let policy = &group.policies[*choice];
        let scenario_utility = if incumbents.contains(&stress_identity(group.role, policy)) {
            policy.lower_utility
        } else {
            policy.upper_utility
        };
        let pool = &pools[policy.pool];
        assignments.push(ScenarioAssignment {
            seat: Seat::ALL[group.role],
            class: group.class,
            row_index: policy.row_index,
            attempt_limit: policy.attempt_limit,
            provider_id: pool.provider_id.clone(),
            plan_id: pool.plan_id.clone(),
            native_harness: policy.native_harness.clone(),
            utility: scenario_utility,
        });
        let nominal_group = nominal_groups
            .iter()
            .position(|nominal| nominal.role == group.role && nominal.class == group.class);
        let Some(nominal_group) = nominal_group else {
            continue;
        };
        let nominal = &nominal_groups[nominal_group].policies[nominal_choices[nominal_group]];
        if policy_identity(group.role, policy) != policy_identity(group.role, nominal) {
            changes.push(CloseChoice {
                seat: Seat::ALL[group.role],
                class: group.class,
                nominal_row_index: nominal.row_index,
                challenger_row_index: policy.row_index,
                nominal_utility: nominal.lower_utility,
                challenger_utility: scenario_utility,
                challenger_attempt_limit: policy.attempt_limit,
                challenger_provider_id: pool.provider_id.clone(),
                challenger_plan_id: pool.plan_id.clone(),
                challenger_native_harness: policy.native_harness.clone(),
                scenario_status: if proven { "optimal" } else { "search_limit" }.to_owned(),
                scenario_bound: bound,
                scenario_relative_gap: relative_gap,
                scenario_proven: proven,
                condition: "Conditional sensitivity alternative; native capability, billing, and a fresh hold must qualify before replanning.".to_owned(),
            });
        }
    }
    (
        CapabilityScenarioReport {
            status: if proven { "optimal" } else { "feasible_search_limit" }.to_owned(),
            proven,
            quality: service,
            baseline_quality: Some(baseline_service),
            bound,
            relative_gap,
            message: "Role-specific incumbent-lower and challenger-upper endpoints form a declared sensitivity scenario, not a statistical joint confidence region. The nominal plan remains funded.".to_owned(),
            assignments,
        },
        changes,
    )
}

fn seed_class_recommendations(
    portfolio: &mut Portfolio,
    candidates: &[Vec<Policy>],
    rows: &[Row],
    settings: &Settings,
) {
    let hours = settings.agent_hours;
    for (role_index, role) in portfolio.roles.iter_mut().enumerate() {
        if matches!(role.seat, Seat::Orchestrator | Seat::NetResearch) {
            continue;
        }
        for rule in &mut role.rules {
            let factor = rule.class.resource_factor();
            let selected = candidates[role_index]
                .iter()
                .filter(|policy| {
                    let pool = &portfolio.pools[policy.pool];
                    pool.rate_fits(
                        policy.cycle.demand(),
                        policy.cycle.reserved_seconds / 3600.0,
                        policy.fable,
                        settings.orchestrators,
                    ) && policy.cycle.reserved_seconds * factor / 3600.0 <= hours
                })
                .max_by(|left, right| {
                    left.utility
                        .total_cmp(&right.utility)
                        .then_with(|| left.competence.total_cmp(&right.competence))
                        .then_with(|| {
                            right
                                .cycle
                                .reserved_usage
                                .total_cmp(&left.cycle.reserved_usage)
                        })
                        .then_with(|| {
                            right
                                .cycle
                                .reserved_seconds
                                .total_cmp(&left.cycle.reserved_seconds)
                        })
                        .then_with(|| right.row_index.cmp(&left.row_index))
                });
            let Some(policy) = selected else {
                rule.message = "No measured route can serve one visit for this class within a single owned account's rate ceilings and working window.".to_owned();
                continue;
            };
            let pool = &portfolio.pools[policy.pool];
            rule.row_index = Some(policy.row_index);
            rule.recommendation_only = true;
            rule.attempt_limit = policy.attempt_limit;
            rule.competence = Some(policy.competence);
            rule.utility = policy.utility;
            rule.evidence_level = policy.evidence_level.clone();
            rule.capability_sensitivity = Some(CapabilityRange {
                nominal: policy.utility,
                lower: policy.lower_utility,
                upper: policy.upper_utility,
            });
            rule.per_call_expected_usage = factor * policy.cycle.nominal_usage;
            rule.per_call_expected_hours = factor * policy.cycle.nominal_seconds / 3600.0;
            rule.per_call_reserved_usage = factor * policy.cycle.reserved_usage;
            rule.per_call_reserved_hours = factor * policy.cycle.reserved_seconds / 3600.0;
            rule.provider_id = Some(pool.provider_id.clone());
            rule.plan_id = Some(pool.plan_id.clone());
            rule.account_claim = format!(
                "Recommendation only; no allocated jobs / {} / {} / {} owned {} / {}",
                pool.provider_id,
                pool.plan_id,
                pool.subscription_count,
                if pool.subscription_count == 1 {
                    "account"
                } else {
                    "accounts"
                },
                crate::agent_setup::binding_id_for(&policy.native_harness, &rows[policy.row_index],)
            );
            rule.fallback_policy = "For an authorized task in this class, jointly replan against all qualified bindings and current native windows; use this evidence-ranked route only if exact capability, billing, and a fresh hold qualify.".to_owned();
            rule.calibration = "Route evidence and per-visit values come from the current benchmark configuration. No job, weekly usage, native capacity, or completion is allocated until an authorized task is jointly admitted.".to_owned();
            rule.fable = policy.fable;
            rule.message = "Recommended route for this class; no jobs are allocated in the current admitted prefix.".to_owned();
        }
    }
}

pub fn research_tier_picks(
    rows: &[Row],
    settings: &Settings,
    vendor_filter: Option<&str>,
) -> Vec<ResearchTierPick> {
    let weights = [0.30, 0.30, 0.40, 0.0];
    crate::types::Tier::ALL
        .into_iter()
        .map(|tier| {
            let budget = match tier {
                crate::types::Tier::Api => None,
                crate::types::Tier::T200 => Some(settings.plan_prices.t200),
                crate::types::Tier::T100 => Some(settings.plan_prices.t100),
                crate::types::Tier::T20 => Some(settings.plan_prices.t20),
            };
            let mut candidates: Vec<_> = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.harness == "model")
                .filter(|(_, row)| row.effort.as_deref() != Some("none"))
                .filter(|(_, row)| vendor_filter.is_none_or(|vendor| row.vendor == vendor))
                .filter_map(|(row_index, row)| {
                    let provider = subscriptions::PROVIDERS
                        .iter()
                        .find(|provider| provider.id == row.vendor);
                    let plan = budget.map(|budget| {
                        provider?
                            .plans
                            .iter()
                            .filter(|plan| plan.monthly_price <= budget)
                            .max_by(|left, right| {
                                left.monthly_price.total_cmp(&right.monthly_price)
                            })
                    });
                    if plan.is_some_and(|plan| plan.is_none()) {
                        return None;
                    }
                    let plan_id = plan
                        .flatten()
                        .map_or("api-analysis", |plan| plan.id)
                        .to_owned();
                    if subscriptions::is_fable(row) && plan_id == "claude-pro" {
                        return None;
                    }
                    let assessment = assess_research(row, weights)?;
                    let native_harness = match (tier, row.vendor.as_str()) {
                        (crate::types::Tier::Api, _) => "Analytical model route",
                        (_, "openai") => "Codex",
                        (_, "anthropic") => "Claude Code",
                        (_, "google") => "Antigravity SDK",
                        (_, "muse") => "Muse Code",
                        (_, "xai") => "Grok Build",
                        _ => return None,
                    };
                    Some(ResearchCandidate {
                        row_index,
                        binding_id: crate::agent_setup::binding_id(row),
                        native_harness: native_harness.to_owned(),
                        provider_id: row.vendor.clone(),
                        plan_id,
                        primary: false,
                        score: assessment.score,
                        accuracy: assessment.accuracy,
                        accuracy_weight: weights[0],
                        non_wrong: assessment.non_wrong,
                        non_wrong_weight: weights[1],
                        conditional_hallucination: assessment.conditional_hallucination,
                        lcr: assessment.lcr,
                        lcr_weight: weights[2],
                        hle: assessment.hle,
                        hle_weight: weights[3],
                        gpqa_diagnostic: row.gpqa,
                        gdp_pdf_diagnostic: row.gdp_pdf,
                        expected_usd: assessment.expected_usd,
                        decode_hours: assessment.decode_hours,
                        source: "Artificial Analysis model evaluation manifest: Standard research policy uses Omniscience accuracy and hallucination rate plus AA-LCR; HLE, GPQA, and GDP.pdf remain diagnostics.".to_owned(),
                        eligibility: if tier == crate::types::Tier::Api {
                            "Analytical API comparison only; no native harness, entitlement, subscription funding, or launch binding is asserted.".to_owned()
                        } else {
                            "Hypothetical plan comparison; exact model entitlement, native research tools, permissions, source scope, billing account, and holds require onboarding.".to_owned()
                        },
                    })
                })
                .collect();
            if let Some(floor) = settings
                .competence_floors
                .get(Seat::NetResearch.name())
                .copied()
                .flatten()
            {
                candidates.retain(|candidate| candidate.score >= floor);
            }
            candidates.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| match (left.expected_usd, right.expected_usd) {
                        (Some(left), Some(right)) => left.total_cmp(&right),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    })
                    .then_with(|| match (left.decode_hours, right.decode_hours) {
                        (Some(left), Some(right)) => left.total_cmp(&right),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    })
                    .then_with(|| left.binding_id.cmp(&right.binding_id))
            });
            if let Some(primary) = candidates.first_mut() {
                primary.primary = true;
            }
            let primary = candidates.first().cloned();
            let mut provider_champions = std::collections::BTreeMap::new();
            for candidate in &candidates {
                provider_champions
                    .entry(candidate.provider_id.clone())
                    .or_insert_with(|| {
                        (
                            candidate.binding_id.clone(),
                            crate::aa::family_key("", &rows[candidate.row_index].model_key),
                        )
                    });
            }
            let frontier: Vec<_> = candidates
                .iter()
                .map(|candidate| {
                    let Some((champion, family)) =
                        provider_champions.get(&candidate.provider_id)
                    else {
                        return false;
                    };
                    candidate.binding_id == *champion
                        || (crate::aa::family_key("", &rows[candidate.row_index].model_key)
                            == *family
                            && !candidates.iter().any(|other| {
                        let resources = other
                            .expected_usd
                            .zip(other.decode_hours)
                            .zip(candidate.expected_usd.zip(candidate.decode_hours));
                        other.provider_id == candidate.provider_id
                            && crate::aa::family_key("", &rows[other.row_index].model_key)
                                == *family
                            && resources.is_some_and(
                                |((other_usd, other_hours),
                                  (candidate_usd, candidate_hours))| {
                                    other.score >= candidate.score
                                        && other_usd <= candidate_usd
                                        && other_hours <= candidate_hours
                                        && (other.score > candidate.score
                                            || other_usd < candidate_usd
                                            || other_hours < candidate_hours)
                                },
                            )
                    }))
                })
                .collect();
            let alternatives = candidates
                .into_iter()
                .zip(frontier)
                .skip(1)
                .filter_map(|(candidate, keep)| keep.then_some(candidate))
                .collect();
            ResearchTierPick {
                tier,
                primary,
                alternatives,
                scope: "Standard research weights: Omniscience accuracy 0.30, no incorrect answer across all questions 0.30, AA-LCR 0.40. This analytical tier comparison has no retry-throughput or coding prerequisite.".to_owned(),
            }
        })
        .collect()
}

pub fn compare_conductors(rows: &[Row], settings: &Settings) -> Vec<ConductorTradeoff> {
    let selected: Vec<_> = subscriptions::PROVIDERS
        .iter()
        .filter_map(|provider| {
            let plan_id = settings.subscriptions.get(provider.id)?;
            Some((
                provider,
                provider.plans.iter().find(|plan| plan.id == plan_id)?,
            ))
        })
        .collect();
    let mut floors: Vec<_> = rows
        .iter()
        .filter(|row| {
            selected.iter().any(|(provider, plan)| {
                subscriptions::eligible(provider, plan, row)
                    || (row.harness == "model"
                        && rows.iter().any(|native| {
                            subscriptions::eligible(provider, plan, native)
                                && crate::aa::family_key("", &native.model_key)
                                    == crate::aa::family_key("", &row.model_key)
                        }))
            })
        })
        .filter_map(|row| {
            engine::orchestrator_dispatch_cycle(row, settings)?;
            selected
                .iter()
                .any(|(provider, plan)| {
                    let maps_to_provider = subscriptions::eligible(provider, plan, row)
                        || (row.harness == "model"
                            && rows.iter().any(|native| {
                                subscriptions::eligible(provider, plan, native)
                                    && crate::aa::family_key("", &native.model_key)
                                        == crate::aa::family_key("", &row.model_key)
                            }));
                    let capacity = plan.weekly_bounds(provider.id, settings).0;
                    maps_to_provider && capacity > 0.0 && settings.agent_hours > 0.0
                })
                .then(|| engine::competence(row, Seat::Orchestrator))
                .flatten()
        })
        .filter(|value| value.is_finite() && *value > 0.0)
        .collect();
    floors.sort_by(f64::total_cmp);
    floors.dedup_by(|left, right| (*left - *right).abs() <= 1e-12);
    let mut points = Vec::new();
    for floor in floors {
        let mut scenario = settings.clone();
        scenario
            .competence_floors
            .insert(Seat::Orchestrator.name().to_owned(), Some(floor));
        let Some(portfolio) = allocate_core(rows, &scenario, false) else {
            continue;
        };
        let Some(conductor) = &portfolio.conductor else {
            continue;
        };
        let point = ConductorTradeoff {
            competence_floor: floor,
            row_index: conductor.row_index,
            binding_id: conductor.binding_id.clone(),
            admitted_changes: portfolio.dispatch.admitted_changes,
            quality: portfolio.dispatch.solver.quality,
            bound: portfolio.dispatch.solver.bound,
            relative_gap: portfolio.dispatch.solver.relative_gap,
            proven: portfolio.dispatch.solver.proven_optimal,
            pool_claims: portfolio
                .pools
                .iter()
                .filter(|pool| pool.reserved_usage > 0.0 || pool.reserved_hours > 0.0)
                .map(|pool| {
                    format!(
                        "{} / {}: {:.2} modeled API-equivalent USD, {:.2} reference h",
                        pool.provider_id, pool.plan_id, pool.reserved_usage, pool.reserved_hours
                    )
                })
                .collect(),
        };
        if !points.iter().any(|existing: &ConductorTradeoff| {
            existing.binding_id == point.binding_id
                && existing.admitted_changes == point.admitted_changes
                && (existing.quality - point.quality).abs() <= 1e-9
        }) {
            points.push(point);
        }
    }
    points
}

pub fn allocate(rows: &[Row], settings: &Settings) -> Option<Portfolio> {
    allocate_core(rows, settings, true)
}

fn allocate_core(
    rows: &[Row],
    settings: &Settings,
    include_sensitivity: bool,
) -> Option<Portfolio> {
    let selected: Vec<_> = subscriptions::PROVIDERS
        .iter()
        .filter_map(|provider| {
            let id = settings.subscriptions.get(provider.id)?;
            Some((provider, provider.plans.iter().find(|plan| plan.id == id)?))
        })
        .collect();
    if selected.is_empty() {
        return None;
    }
    let capacity = |provider: &subscriptions::Provider, plan: &subscriptions::Plan| {
        plan.weekly_bounds(provider.id, settings).0
    };
    let mut portfolio = Portfolio {
        orchestrators: settings.orchestrators, available_hours_per_provider: settings.agent_hours, total_scheduled_hours: 0.0, total_agent_hours: 0.0,
        monthly_price: selected.iter().map(|(provider, plan)| plan.monthly_price * settings.subscription_count(provider.id) as f64).sum(),
        roles: Seat::ALL.into_iter().map(|seat| RoleAllocation {
            seat, allocated_hours: 0.0, quality_utility: 0.0,
            rules: WorkClass::ALL.into_iter().map(|class| DispatchRule {
                class, policy_id: format!("{}-{}-{}", crate::team_policy::VERSION, seat_id(seat), class.name().to_ascii_lowercase()), planned_jobs: 0, recommendation_only: false, account_claim: if seat == Seat::NetResearch { "Conditional native binding" } else { "Unfunded" }.to_owned(), fallback_policy: "Defer and jointly replan when no exact qualified binding has a native hold.".to_owned(), calibration: "No qualified evidence or native calibration is available.".to_owned(), condition: class.condition().to_owned(), row_index: None, attempt_limit: 0, lower_effort: None, within_provider_alternatives: Vec::new(), surplus_alternatives: Vec::new(), research_candidates: Vec::new(),
                expected_visits: 0.0, reserved_visits: 0, expected_completions: 0.0, competence: None, utility: 0.0,
                nominal_usage: 0.0, nominal_hours: 0.0, reserved_usage: 0.0, reserved_hours: 0.0,
                per_call_expected_usage: 0.0, per_call_expected_hours: 0.0, per_call_reserved_usage: 0.0, per_call_reserved_hours: 0.0, provider_id: None, plan_id: None,
                fable: false, failure_action: failure_action(seat).to_owned(), message: if seat == Seat::NetResearch { "On demand; exact research tools, source scope, role evidence, billing, and native holds must qualify for the ready job." } else { "No funded static assignment is available for this workflow." }.to_owned(), evidence_level: String::new(), capability_sensitivity: None, repair_activation_proxy: None,
            }).collect(),
        }).collect(),
        pools: selected.iter().map(|(provider, plan)| PoolUsage {
            provider_id: provider.id.to_owned(), subscription_count: settings.subscription_count(provider.id), per_account_weekly_capacity: capacity(provider, plan), per_account_fable_capacity: plan.fable_capacity(provider.id, settings), windows: plan.resolved_windows(provider.id, settings), rate_windows: Vec::new(), rate_infeasible: false, provider_name: provider.name.to_owned(), plan_id: plan.id.to_owned(), plan_name: plan.name.to_owned(), monthly_price: plan.monthly_price * settings.subscription_count(provider.id) as f64, price_is_estimate: plan.price_is_estimate,
            weekly_capacity: capacity(provider, plan), nominal_usage: 0.0, stress_usage: 0.0, remaining_capacity: capacity(provider, plan), utilization_pct: 0.0,
            available_hours: settings.agent_hours, scheduled_hours: 0.0, unused_hours: settings.agent_hours, reserved_usage: 0.0, reserved_hours: 0.0, remaining_reserved_capacity: capacity(provider, plan), remaining_reserved_hours: settings.agent_hours,
            unit: format!("{} API-equivalent USD", plan.weekly_basis(provider.id, settings)), source_url: plan.source_url.to_owned(), allowance_basis: format!("{} Owned subscriptions: {}; forecast uses one bound account. Every non-reference window constrains known demand in its own unit. Unknown native counts cannot constrain USD demand.", plan.capacity_summary(provider.id, settings), settings.subscription_count(provider.id)),
        }).collect(),
        limits: selected.iter().filter(|(provider, plan)| provider.id == "anthropic" && plan.id != "claude-pro").map(|(provider, plan)| PoolLimit {
            provider_id: provider.id.to_owned(), name: format!("Fable: published 50% of weekly-usd; {} USD amount", plan.weekly_basis(provider.id, settings)), weekly_capacity: plan.fable_capacity(provider.id, settings), nominal_usage: 0.0, stress_usage: 0.0, reserved_usage: 0.0, remaining_capacity: plan.fable_capacity(provider.id, settings),
        }).collect(),
        assumptions: vec![
            "Capacity table: anecdotal USD amounts and published quota rules are labeled in Settings. Every non-reference window at every nesting level enforces parallel orchestrators times aggregate committed seat rate <= cap / reset hours. Native counts and USD are separate demand dimensions; unknown native counts need measurement.".to_owned(),
            "Demand is weekly hours times parallel orchestrators divided by the class-mix-weighted critical path of the fastest eligible measured workflow stages, including the full conditional repair path. Only complete bundles enter integer admission; fractional demand remains in jobs_per_week. Parallelism multiplies demand and committed seat rates, never quota. The existing class mix and resource factors are unchanged.".to_owned(),
            "The profile binds one account per provider, so the forecast uses one modeled allowance and one account calendar per provider regardless of owned subscription count. Additional subscriptions remain in purchase costs but contribute no forecast capacity. Forecast account ordinals are scheduling slots, not native account identities, authentication, meters, or authorization.".to_owned(),
            "Select workflow templates from consequence, uncertainty, coupling, reversibility, evidence need, tool risk, correlation, and deadline risk. File count is load information only and never determines the workflow.".to_owned(),
            "The full forecast uses largest-remainder class counts with canonical ties. A fixed deficit-ordered sequence supplies nested admission prefixes; reduced demand never re-apportions.".to_owned(),
            "The API-equivalent worker proxy uses one stage for Focused, six for Standard, and seven for Complex and Extensive work. Expected repair activation uses the selected Implementer's bounded-attempt benchmark residual as an uncalibrated proxy; the full conditional repair path remains reserved. Each class can explicitly include one AA research-reference visit; the default coding forecast excludes it. Native tasks expand from their risk vectors and actual task evidence.".to_owned(),
            "The Orchestrator uses one persistent model and effort across the whole plan. Its ongoing demand is not inferred from worker jobs. Optional USD and hour headroom is reserved once on its selected account; every actual native call is charged through the runtime ledger. Each worker visit uses one model and effort with 1–3 same-model attempts.".to_owned(),
            "Multi-criteria role utility uses each skill component once: capped completion for reference-workload evidence, static values for other skills. It is not real-job success probability. Class factors scale demand and resources, not calibrated difficulty.".to_owned(),
            "The proxy calendar places its declared stage sequence in earliest account gaps to estimate API-equivalent capacity. Native scheduling instead uses authorized ready task IDs, risk-selected DAGs, immutable artifact dependencies, current reset windows, and deadlines.".to_owned(),
            "The solver optimizes only assignments feasible under this declared scheduler. Modeled allowances and full-cap buffers do not guarantee real vendor quota or runtime; unknown actual usage requires pause and reconciliation.".to_owned(),
        ],
        message: String::new(),
        math_audit: MathAudit {
            objective: "Multi-criteria role utility uses each competence component once: capped completion replaces reference-workload evidence, while other skill components remain static. Competence floors apply separately; this is not a calibrated probability of completing a real job.".to_owned(),
            coordination_proxy: "Persistent orchestration competence is the Intelligence Index. LCR and HLE are diagnostics and do not enter the score. Ongoing usage is unknown; optional fixed headroom is reserved once and every actual native call is charged through the runtime ledger.".to_owned(),
            ..MathAudit::default()
        },
        conductor: None,
        close_choices: Vec::new(),
        capability_scenario: None,
        dispatch: DispatchPlan {
            policy_version: crate::team_policy::VERSION.to_owned(),
            forecast_changes: 0, admitted_changes: 0, deferred_changes: 0, cadence_hours: None, jobs_per_week: None, repair_incidence: 0.0,
            classes: WorkClass::ALL.into_iter().map(|class| WorkClassDemand { class, condition: class.condition().to_owned(), resource_factor: class.resource_factor(), forecast_changes: 0, admitted_changes: 0, research_included: settings.research_classes.get(class.name()).copied().unwrap_or(false) }).collect(),
            solver: SolverReport { status: "infeasible".to_owned(), proven_optimal: false, quality: 0.0, bound: None, relative_gap: None, nodes: 0, message: String::new() },
            executable: false, admitted_sequence: Vec::new(), reserved_makespan_hours: 0.0, schedule: Vec::new(), repair_sensitivity: Vec::new(), service: TeamServiceReport::default(),
        },
    };
    if let Some(role) = portfolio
        .roles
        .iter_mut()
        .find(|role| role.seat == Seat::NetResearch)
    {
        for rule in &mut role.rules {
            let weights = match rule.class {
                WorkClass::Focused => [0.50, 0.30, 0.20, 0.0],
                WorkClass::Standard => [0.30, 0.30, 0.40, 0.0],
                WorkClass::Complex => [0.20, 0.20, 0.40, 0.20],
                WorkClass::Extensive => [0.15, 0.25, 0.40, 0.20],
            };
            let mut bindings = std::collections::BTreeSet::new();
            let mut candidates: Vec<_> = rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.harness == "model")
                .filter_map(|(row_index, row)| {
                    let (provider, plan) = selected.iter().find(|(provider, plan)| {
                        row.vendor == provider.id
                            && row.effort.as_deref() != Some("none")
                            && (provider.id != "anthropic"
                                || !subscriptions::is_fable(row)
                                || plan.id != "claude-pro")
                    })?;
                    let native_harness = rows.iter().find_map(|native| {
                        (native.harness != "model"
                            && subscriptions::eligible(provider, plan, native))
                            .then(|| native.harness.clone())
                    }).or_else(|| match provider.id {
                        "openai" => Some("Codex".to_owned()),
                        "anthropic" => Some("Claude Code".to_owned()),
                        "google" => Some("Antigravity SDK".to_owned()),
                        "muse" => Some("Muse Code".to_owned()),
                        "xai" => Some("Grok Build".to_owned()),
                        _ => None,
                    })?;
                    let assessment = assess_research(row, weights)?;
                    let binding_id = crate::agent_setup::binding_id_for(&native_harness, row);
                    bindings.insert(binding_id.clone()).then_some(ResearchCandidate {
                        row_index,
                        binding_id,
                        native_harness,
                        provider_id: provider.id.to_owned(),
                        plan_id: plan.id.to_owned(),
                        primary: false,
                        score: assessment.score,
                        accuracy: assessment.accuracy,
                        accuracy_weight: weights[0],
                        non_wrong: assessment.non_wrong,
                        non_wrong_weight: weights[1],
                        conditional_hallucination: assessment.conditional_hallucination,
                        lcr: assessment.lcr,
                        lcr_weight: weights[2],
                        hle: assessment.hle,
                        hle_weight: weights[3],
                        gpqa_diagnostic: row.gpqa,
                        gdp_pdf_diagnostic: row.gdp_pdf,
                        expected_usd: assessment.expected_usd,
                        decode_hours: assessment.decode_hours,
                        source: "Artificial Analysis model evaluation manifest: Omniscience accuracy and hallucination rate, AA-LCR, HLE, GPQA, GDP.pdf, and canonical benchmark token resources; model-level evidence transferred to the separately named native harness.".to_owned(),
                        eligibility: "Exact model and effort evidence only. Native research tools, permissions, source scope, subscription billing, role evidence, and a native hold must qualify for this task.".to_owned(),
                    })
                })
                .collect();
            let floor = settings
                .competence_floors
                .get(Seat::NetResearch.name())
                .copied()
                .flatten()
                .unwrap_or(0.0);
            candidates.retain(|candidate| candidate.score >= floor);
            candidates.sort_by(|left, right| {
                right
                    .score
                    .total_cmp(&left.score)
                    .then_with(|| match (left.expected_usd, right.expected_usd) {
                        (Some(left), Some(right)) => left.total_cmp(&right),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    })
                    .then_with(|| match (left.decode_hours, right.decode_hours) {
                        (Some(left), Some(right)) => left.total_cmp(&right),
                        (Some(_), None) => std::cmp::Ordering::Less,
                        (None, Some(_)) => std::cmp::Ordering::Greater,
                        (None, None) => std::cmp::Ordering::Equal,
                    })
                    .then_with(|| left.binding_id.cmp(&right.binding_id))
            });
            if let Some(primary) = candidates.first_mut() {
                primary.primary = true;
            }
            rule.research_candidates = candidates;
            let (condition, fallback) = match rule.class {
                WorkClass::Focused => (
                    "Known authoritative source fetch with direct support, freshness, and consuming-decision provenance.",
                    "Require an exact qualified known-source fetch route; otherwise provide the source locally, reduce scope, or defer.",
                ),
                WorkClass::Standard => (
                    "Targeted search and source comparison with direct support, freshness, and uncertainty recorded per claim.",
                    "Require exact qualified search and fetch across the approved source scope; otherwise narrow the question or defer.",
                ),
                WorkClass::Complex => (
                    "Multi-source synthesis that resolves material contradictions and preserves claim-level source and uncertainty lineage.",
                    "Require an exact qualified synthesis route and explicit contradiction handling; otherwise split the question or defer.",
                ),
                WorkClass::Extensive => (
                    "Bounded independent searches, source-quality review, contradiction resolution, and an acceptance-ready evidence packet.",
                    "Require a qualified route whose bounded task scope includes independent searches and source-quality synthesis review; otherwise reduce scope or defer.",
                ),
            };
            rule.condition = condition.to_owned();
            rule.fallback_policy = fallback.to_owned();
            rule.calibration = "AA model-level research metrics are measured in their published benchmark configurations. Transfer to the separately named native research harness, exact tool capability, entitlement, native consumption, duration, and outcomes remain unverified until onboarding and real-work telemetry.".to_owned();
            rule.message = if rule.research_candidates.is_empty() {
                if floor > 0.0 {
                    format!(
                        "No complete subscribed research configuration meets the configured {:.3} class score minimum.",
                        floor
                    )
                } else {
                    "No subscribed model configuration has every research component required by this class and a supported native provider harness.".to_owned()
                }
            } else {
                "Conditional candidates; runtime qualification and native admission remain required.".to_owned()
            };
        }
    }
    let mut candidates: Vec<Vec<Policy>> = (0..Seat::ALL.len()).map(|_| Vec::new()).collect();
    for (row_index, row) in rows.iter().enumerate() {
        let native_pool = selected
            .iter()
            .position(|(provider, plan)| subscriptions::eligible(provider, plan, row));
        let model_binding = (row.harness == "model")
            .then(|| {
                selected
                    .iter()
                    .enumerate()
                    .find_map(|(pool, (provider, plan))| {
                        rows.iter().find_map(|native| {
                            (subscriptions::eligible(provider, plan, native)
                                && crate::aa::family_key("", &native.model_key)
                                    == crate::aa::family_key("", &row.model_key))
                            .then_some((pool, native.harness.clone()))
                        })
                    })
            })
            .flatten();
        let orchestrator_pool = native_pool.or(model_binding.as_ref().map(|(pool, _)| *pool));
        let cycles = [
            engine::dispatch_cycles(row, Seat::Implementer, settings),
            engine::dispatch_cycles(row, Seat::Reviewer, settings),
        ];
        for (role, seat) in Seat::ALL.into_iter().enumerate() {
            if seat == Seat::NetResearch {
                continue;
            }
            let floor = settings
                .competence_floors
                .get(seat.name())
                .copied()
                .flatten()
                .unwrap_or(0.0);
            let pool = if seat == Seat::Orchestrator {
                orchestrator_pool
            } else {
                native_pool
            };
            let Some(pool) = pool.filter(|pool| portfolio.pools[*pool].weekly_capacity > 0.0)
            else {
                continue;
            };
            let competence = if seat == Seat::Orchestrator {
                engine::orchestrator_capability(row)
            } else {
                engine::competence(row, seat)
            };
            let Some(competence) = competence.filter(|value| {
                value.is_finite() && (0.0..=1.0).contains(value) && *value >= floor && *value > 0.0
            }) else {
                continue;
            };
            if seat == Seat::Orchestrator {
                let fable = subscriptions::is_fable(row) && row.vendor == "anthropic";
                if !conductor_headroom_fits(&portfolio.pools[pool], fable, settings) {
                    continue;
                }
                let Some(cycle) = engine::orchestrator_dispatch_cycle(row, settings) else {
                    continue;
                };
                let Some(sensitivity) = engine::orchestrator_capability_sensitivity(row, settings)
                else {
                    continue;
                };
                let Some((utility, lower_utility, upper_utility)) = sensitivity
                    .nominal
                    .zip(sensitivity.low)
                    .zip(sensitivity.high)
                    .map(|((nominal, lower), upper)| (nominal, lower, upper))
                    .filter(|_| !sensitivity.incomplete)
                else {
                    continue;
                };
                candidates[role].push(Policy {
                    row_index,
                    native_harness: model_binding
                        .as_ref()
                        .map(|(_, harness)| harness.clone())
                        .unwrap_or_else(|| row.harness.clone()),
                    pool,
                    competence,
                    utility,
                    attempt_limit: 1,
                    cycle,
                    fable,
                    research_class: None,
                    lower_utility,
                    upper_utility,
                    evidence_level: role_evidence_level(row, seat),
                });
                continue;
            }
            let Some(cycles) =
                cycles[usize::from(!matches!(seat, Seat::Implementer | Seat::Debugger))].as_ref()
            else {
                continue;
            };
            for (index, cycle) in cycles.iter().copied().enumerate() {
                if cycle.success > 0.0
                    && let Some(utility) = engine::dispatch_utility(row, seat, &cycle)
                    && let Some(sensitivity) =
                        engine::dispatch_utility_sensitivity(row, seat, &cycle, settings)
                    && !sensitivity.incomplete
                {
                    let utility = sensitivity.nominal.unwrap_or(utility);
                    candidates[role].push(Policy {
                        row_index,
                        native_harness: row.harness.clone(),
                        pool,
                        competence,
                        utility,
                        attempt_limit: index + 1,
                        cycle,
                        fable: subscriptions::is_fable(row) && row.vendor == "anthropic",
                        research_class: None,
                        lower_utility: sensitivity.low.unwrap_or(utility),
                        upper_utility: sensitivity.high.unwrap_or(utility),
                        evidence_level: role_evidence_level(row, seat),
                    });
                }
            }
        }
    }
    let research_role = Seat::ALL
        .iter()
        .position(|seat| *seat == Seat::NetResearch)
        .unwrap_or(Seat::ALL.len() - 1);
    let stress = 1.0 + settings.assumption_span_pct / 100.0;
    if let Some(role) = portfolio
        .roles
        .iter()
        .find(|role| role.seat == Seat::NetResearch)
    {
        for rule in role.rules.iter().filter(|rule| {
            settings
                .research_classes
                .get(rule.class.name())
                .copied()
                .unwrap_or(false)
        }) {
            for candidate in &rule.research_candidates {
                let sensitivity = research_sensitivity(
                    candidate,
                    (settings.assumption_span_pct / 100.0).clamp(0.0, 1.0),
                );
                let Some(pool) = portfolio.pools.iter().position(|pool| {
                    pool.provider_id == candidate.provider_id && pool.plan_id == candidate.plan_id
                }) else {
                    continue;
                };
                let Some((usage, hours)) = candidate.expected_usd.zip(candidate.decode_hours)
                else {
                    continue;
                };
                candidates[research_role].push(Policy {
                    row_index: candidate.row_index,
                    native_harness: candidate.native_harness.clone(),
                    pool,
                    competence: candidate.score,
                    utility: candidate.score,
                    attempt_limit: 1,
                    cycle: DispatchCycle {
                        success: 0.0,
                        nominal_usage: usage,
                        nominal_seconds: hours * 3600.0,
                        reserved_usage: usage * stress,
                        reserved_seconds: hours * 3600.0 * stress,
                        native_demand: CapacityDemand::default(),
                        reference_completion: [None; 3],
                    },
                    fable: subscriptions::is_fable(&rows[candidate.row_index])
                        && candidate.provider_id == "anthropic",
                    research_class: Some(rule.class),
                    lower_utility: sensitivity.lower,
                    upper_utility: sensitivity.upper,
                    evidence_level: research_evidence_level(&rows[candidate.row_index], rule.class),
                });
            }
        }
    }
    let Some(path_hours) = reference_path_hours(&candidates, settings) else {
        portfolio.message = "Workflow demand unknown: every required DAG stage needs an eligible policy with a measured duration.".to_owned();
        refresh_rate_usage(&mut portfolio, rows, settings);
        return Some(portfolio);
    };
    let jobs_per_week = settings.agent_hours * settings.orchestrators as f64 / path_hours;
    let forecast = (0..)
        .take_while(|index| (*index + 1) as f64 <= jobs_per_week)
        .count();
    let sequence = sequence(forecast);
    portfolio.dispatch.cadence_hours = Some(path_hours);
    portfolio.dispatch.jobs_per_week = Some(jobs_per_week);
    portfolio.dispatch.forecast_changes = forecast;
    portfolio.dispatch.deferred_changes = forecast;
    for class in &mut portfolio.dispatch.classes {
        class.forecast_changes = sequence
            .iter()
            .filter(|value| **value == class.class)
            .count();
    }
    for (pool_index, pool) in portfolio.pools.iter_mut().enumerate() {
        let worker_policies: Vec<_> = candidates
            .iter()
            .zip(Seat::ALL)
            .filter(|(_, seat)| *seat != Seat::Orchestrator)
            .flat_map(|(policies, _)| policies)
            .filter(|policy| policy.pool == pool_index)
            .collect();
        pool.rate_infeasible = !worker_policies.is_empty()
            && worker_policies.iter().all(|policy| {
                !pool.rate_fits(
                    policy.cycle.demand(),
                    policy.cycle.reserved_seconds / 3600.0,
                    policy.fable,
                    settings.orchestrators,
                )
            });
    }
    let all_candidates = candidates.clone();
    let candidates: Vec<_> = candidates
        .into_iter()
        .map(|policies| prune(policies, rows, settings.monotonic_class_competence))
        .collect();
    seed_class_recommendations(&mut portfolio, &candidates, rows, settings);
    let mut nodes = 0;
    let mut work_units = 0;
    let mut searches = Vec::new();
    let mut numerical_failure = None;
    let mut next_count = 1;
    while next_count <= forecast && work_units < SEARCH_NODES {
        let result = search(
            next_count,
            &sequence,
            &candidates,
            &portfolio.pools,
            settings,
            1,
            None,
        );
        nodes += result.state.nodes();
        work_units += 1;
        numerical_failure = numerical_failure.or(result.state.limitation());
        searches.push(result);
        next_count += 1;
    }
    while work_units < SEARCH_NODES {
        let Some(index) = searches
            .iter()
            .enumerate()
            .filter(|(_, search)| search.state.can_advance())
            .max_by(|(_, left), (_, right)| {
                search_service_bound(left)
                    .total_cmp(&search_service_bound(right))
                    .then_with(|| left.count.cmp(&right.count))
            })
            .map(|(index, _)| index)
        else {
            break;
        };
        let slice = (SEARCH_NODES - work_units).min(32);
        let search = &mut searches[index];
        let previous_nodes = search.state.nodes();
        solver::advance(
            &search.problem,
            &mut search.state,
            slice,
            false,
            |choices| {
                schedule(
                    &search.groups,
                    choices,
                    &sequence[..search.count],
                    &portfolio.pools,
                    settings,
                )
            },
        );
        let progressed = search.state.nodes().saturating_sub(previous_nodes);
        nodes += progressed;
        work_units += slice;
        numerical_failure = numerical_failure.or(search.state.limitation());
    }
    let selected = searches
        .iter()
        .enumerate()
        .filter_map(|(index, search)| search_service(search).map(|service| (index, service)))
        .max_by(|(left_index, left), (right_index, right)| {
            left.total_cmp(right)
                .then_with(|| {
                    searches[*left_index]
                        .count
                        .cmp(&searches[*right_index].count)
                })
                .then_with(|| {
                    let left_solution = searches[*left_index]
                        .state
                        .incumbent()
                        .expect("feasible prefix");
                    let right_solution = searches[*right_index]
                        .state
                        .incumbent()
                        .expect("feasible prefix");
                    right_solution
                        .nominal_cost
                        .total_cmp(&left_solution.nominal_cost)
                        .then_with(|| {
                            right_solution
                                .nominal_time
                                .total_cmp(&left_solution.nominal_time)
                        })
                })
        });
    let maximum_volume = searches
        .iter()
        .filter(|search| search.state.has_solution())
        .max_by_key(|search| search.count);
    let Some((_, _)) = selected else {
        let worker_infeasible = next_count > forecast
            && !searches.is_empty()
            && searches.iter().all(|search| search.state.proven());
        portfolio.dispatch.solver.nodes = nodes;
        portfolio.dispatch.solver.status = if forecast == 0 {
            "no_worker_demand"
        } else if worker_infeasible {
            "infeasible"
        } else if numerical_failure.is_some() {
            "solver_failure"
        } else {
            "search_limit"
        }
        .to_owned();
        portfolio.message = if forecast == 0 {
            "No worker demand is declared. Persistent Orchestrator selection remains available on demand.".to_owned()
        } else if worker_infeasible {
            "No complete worker plan is feasible under the declared rate ceilings and calendar resources. Persistent Orchestrator selection remains independent of worker demand.".to_owned()
        } else {
            "No complete worker plan was found within the bounded service search. Persistent Orchestrator selection remains independent of worker demand.".to_owned()
        };
        portfolio.dispatch.solver.message = portfolio.message.clone();
        let orchestrator_role = Seat::ALL
            .iter()
            .position(|seat| *seat == Seat::Orchestrator)
            .unwrap_or(0);
        if let Some(policy) = candidates[orchestrator_role]
            .iter()
            .filter(|policy| {
                conductor_headroom_fits(&portfolio.pools[policy.pool], policy.fable, settings)
            })
            .max_by(|left, right| {
                left.utility.total_cmp(&right.utility).then_with(|| {
                    right
                        .cycle
                        .nominal_usage
                        .total_cmp(&left.cycle.nominal_usage)
                })
            })
        {
            let pool = &mut portfolio.pools[policy.pool];
            pool.reserved_usage += settings.orchestrator_headroom_usd.unwrap_or(0.0);
            pool.reserved_hours += settings.orchestrator_headroom_hours.unwrap_or(0.0);
            if policy.fable {
                for limit in portfolio
                    .limits
                    .iter_mut()
                    .filter(|limit| limit.provider_id == pool.provider_id)
                {
                    limit.reserved_usage += settings.orchestrator_headroom_usd.unwrap_or(0.0);
                    limit.stress_usage = limit.reserved_usage;
                    limit.remaining_capacity =
                        (limit.weekly_capacity - limit.reserved_usage).max(0.0);
                }
            }
            portfolio.conductor = Some(ConductorAllocation {
                row_index: policy.row_index,
                binding_id: crate::agent_setup::binding_id_for(
                    &policy.native_harness,
                    &rows[policy.row_index],
                ),
                native_harness: policy.native_harness.clone(),
                provider_id: pool.provider_id.clone(),
                plan_id: pool.plan_id.clone(),
                attempt_limit: 1,
                competence: policy.competence,
                utility: policy.utility,
                persistent: true,
                evidence_profile: "Intelligence Index".to_owned(),
                evidence_level: policy.evidence_level.clone(),
                capability_sensitivity: Some(CapabilityRange {
                    nominal: policy.utility,
                    lower: policy.lower_utility,
                    upper: policy.upper_utility,
                }),
                usage_forecast: None,
                headroom: ConductorHeadroom {
                    usd: settings.orchestrator_headroom_usd,
                    hours: settings.orchestrator_headroom_hours,
                },
                per_call_expected_usage: policy.cycle.nominal_usage,
                per_call_expected_hours: policy.cycle.nominal_seconds / 3600.0,
                per_call_reserved_usage: policy.cycle.reserved_usage,
                per_call_reserved_hours: policy.cycle.reserved_seconds / 3600.0,
                cost_basis: rows[policy.row_index]
                    .orchestrator_cost_basis
                    .clone()
                    .unwrap_or_default(),
                time_basis: rows[policy.row_index]
                    .orchestrator_time_basis
                    .clone()
                    .unwrap_or_default(),
                classes: WorkClass::ALL
                    .into_iter()
                    .map(|class| ConductorClassDemand {
                        class,
                        changes: 0,
                        resource_factor: class.resource_factor(),
                        visits: 0,
                    })
                    .collect(),
            });
        }
        for pool in &mut portfolio.pools {
            pool.stress_usage = pool.reserved_usage;
            pool.remaining_reserved_capacity =
                (pool.weekly_capacity - pool.reserved_usage).max(0.0);
            pool.remaining_reserved_hours = (pool.available_hours - pool.reserved_hours).max(0.0);
            pool.utilization_pct = if pool.weekly_capacity > 0.0 {
                100.0 * pool.reserved_usage / pool.weekly_capacity
            } else {
                0.0
            };
        }
        adaptive_routes(&mut portfolio, &candidates, rows);
        fill_math_audit(&mut portfolio);
        refresh_rate_usage(&mut portfolio, rows, settings);
        return Some(portfolio);
    };
    let maximum_volume_admitted = maximum_volume.map_or(0, |search| search.count);
    let maximum_volume_service = maximum_volume.and_then(search_service);
    let unsearched_bound = (next_count..=forecast)
        .map(|count| {
            sequence[..count]
                .iter()
                .map(|class| class.resource_factor())
                .sum::<f64>()
        })
        .reduce(f64::max)
        .unwrap_or(0.0);
    let global_bound = searches
        .iter()
        .map(search_service_bound)
        .reduce(f64::max)
        .unwrap_or(0.0)
        .max(unsearched_bound);
    let all_proven = next_count > forecast && searches.iter().all(|search| search.state.proven());
    let (selected_index, best_service) = selected.expect("selected service");
    let best = searches.swap_remove(selected_index);
    let best_count = best.count;
    let best_workload = best.workload;
    let best_groups = best.groups;
    let outcome = solver::finish(best.state);
    portfolio.dispatch.solver.nodes = nodes;
    let solution = outcome.solution.expect("selected dispatch solution");
    let proven = all_proven && global_bound <= best_service + 1e-9;
    portfolio.dispatch.admitted_changes = best_count;
    portfolio.dispatch.deferred_changes = forecast - best_count;
    portfolio.dispatch.executable = true;
    portfolio.dispatch.admitted_sequence = sequence[..best_count].to_vec();
    portfolio.dispatch.reserved_makespan_hours = solution.payload.makespan;
    portfolio.dispatch.schedule = solution.payload.visits;
    for class in &sequence[..best_count] {
        portfolio.dispatch.classes[class.index()].admitted_changes += 1;
    }
    portfolio.dispatch.solver.status = if proven {
        "optimal"
    } else if numerical_failure.is_some() {
        "feasible_numerical_limit"
    } else {
        "feasible_search_limit"
    }
    .to_owned();
    portfolio.dispatch.solver.proven_optimal = proven;
    portfolio.dispatch.solver.quality = best_service;
    portfolio.dispatch.solver.bound = Some(global_bound.max(best_service));
    portfolio.dispatch.solver.relative_gap =
        Some((global_bound / best_service.max(f64::MIN_POSITIVE) - 1.0).max(0.0));
    portfolio.dispatch.solver.message = if proven {
        "The quality-adjusted service objective is optimal across every forecast prefix within the declared scheduler and numerical tolerances.".to_owned()
    } else {
        "The selected worker plan has the best quality-adjusted service found across bounded prefix searches; global optimality is unresolved.".to_owned()
    };
    portfolio.dispatch.service = TeamServiceReport {
        workload: best_workload,
        team_capability: best_service / best_workload,
        quality_adjusted_service: best_service,
        maximum_volume_admitted,
        maximum_volume_service,
        proven_optimal: proven,
        bound: Some(global_bound.max(best_service)),
        relative_gap: portfolio.dispatch.solver.relative_gap,
        message: "Service combines persistent coordination capability with each class's unique required worker roles; it is a policy utility index, not a task-success probability.".to_owned(),
    };
    if include_sensitivity {
        let (scenario, close_choices) = capability_scenario(
            best_count,
            &sequence,
            &all_candidates,
            &portfolio.pools,
            settings,
            &best_groups,
            &solution.choices,
        );
        portfolio.capability_scenario = Some(scenario);
        portfolio.close_choices = close_choices;
    }
    let mut repair_by_class = [0.0; 4];
    for (group, choice) in best_groups.iter().zip(&solution.choices) {
        if Seat::ALL[group.role] == Seat::Implementer
            && let Some(class) = group.class
        {
            repair_by_class[class.index()] =
                (1.0 - group.policies[*choice].cycle.success).clamp(0.0, 1.0);
        }
    }
    let repair_span = (settings.assumption_span_pct / 100.0).clamp(0.0, 1.0);
    portfolio.dispatch.repair_sensitivity = WorkClass::ALL
        .into_iter()
        .filter(|class| portfolio.dispatch.classes[class.index()].admitted_changes > 0)
        .map(|class| {
            let nominal = if class == WorkClass::Focused {
                0.0
            } else {
                repair_by_class[class.index()]
            };
            ClassRepairSensitivity {
                class,
                nominal,
                lower: (nominal * (1.0 - repair_span)).clamp(0.0, 1.0),
                upper: (nominal * (1.0 + repair_span)).clamp(0.0, 1.0),
                basis: if class == WorkClass::Focused {
                    "Focused workflow has no conditional repair stage.".to_owned()
                } else {
                    "Uncalibrated selected-Implementer bounded-attempt benchmark residual; full conditional repair and rechecks remain reserved.".to_owned()
                },
            }
        })
        .collect();
    let repair_jobs: usize = portfolio
        .dispatch
        .classes
        .iter()
        .filter(|demand| demand.class != WorkClass::Focused)
        .map(|demand| demand.admitted_changes)
        .sum();
    portfolio.dispatch.repair_incidence = if repair_jobs == 0 {
        0.0
    } else {
        portfolio
            .dispatch
            .classes
            .iter()
            .filter(|demand| demand.class != WorkClass::Focused)
            .map(|demand| demand.admitted_changes as f64 * repair_by_class[demand.class.index()])
            .sum::<f64>()
            / repair_jobs as f64
    };
    for (group, choice) in best_groups.iter().zip(solution.choices) {
        let policy = &group.policies[choice];
        let classes: Vec<_> = group
            .class
            .map_or_else(|| WorkClass::ALL.to_vec(), |class| vec![class]);
        for class in classes {
            let changes = portfolio.dispatch.classes[class.index()].admitted_changes;
            let visits = if group.class.is_none() {
                0
            } else {
                reserved_visits(
                    class,
                    Seat::ALL[group.role],
                    settings
                        .research_classes
                        .get(class.name())
                        .copied()
                        .unwrap_or(false),
                )
            };
            let expected = if group.class.is_none() {
                0.0
            } else {
                changes as f64
                    * expected_visits(
                        class,
                        Seat::ALL[group.role],
                        settings
                            .research_classes
                            .get(class.name())
                            .copied()
                            .unwrap_or(false),
                        repair_by_class[class.index()],
                    )
            };
            let factor = class.resource_factor();
            let pool = &mut portfolio.pools[policy.pool];
            let seat = portfolio.roles[group.role].seat;
            let rule = &mut portfolio.roles[group.role].rules[class.index()];
            rule.row_index = Some(policy.row_index);
            if seat == Seat::NetResearch
                && let Some(index) = rule.research_candidates.iter().position(|candidate| {
                    candidate.row_index == policy.row_index
                        && candidate.provider_id == pool.provider_id
                        && candidate.native_harness == policy.native_harness
                })
            {
                for candidate in &mut rule.research_candidates {
                    candidate.primary = false;
                }
                rule.research_candidates[index].primary = true;
                rule.research_candidates.swap(0, index);
            }
            rule.recommendation_only = false;
            rule.policy_id = format!(
                "{}-{}-{}",
                crate::team_policy::VERSION,
                seat_id(seat),
                class.name().to_ascii_lowercase()
            );
            rule.planned_jobs = changes;
            rule.attempt_limit = policy.attempt_limit;
            rule.expected_visits = expected;
            rule.reserved_visits = changes * visits;
            rule.expected_completions = if group.class.is_none() {
                0.0
            } else {
                expected * factor * policy.cycle.success
            };
            rule.competence = Some(policy.competence);
            rule.utility = policy.utility;
            rule.evidence_level = policy.evidence_level.clone();
            rule.capability_sensitivity = Some(CapabilityRange {
                nominal: policy.utility,
                lower: policy.lower_utility,
                upper: policy.upper_utility,
            });
            rule.repair_activation_proxy = (group.class.is_some() && class != WorkClass::Focused)
                .then_some(repair_by_class[class.index()]);
            rule.nominal_usage = expected * factor * policy.cycle.nominal_usage;
            rule.nominal_hours = expected * factor * policy.cycle.nominal_seconds / 3600.0;
            rule.per_call_expected_usage = factor * policy.cycle.nominal_usage;
            rule.per_call_expected_hours = factor * policy.cycle.nominal_seconds / 3600.0;
            rule.per_call_reserved_usage = factor * policy.cycle.reserved_usage;
            rule.per_call_reserved_hours = factor * policy.cycle.reserved_seconds / 3600.0;
            rule.reserved_usage = rule.reserved_visits as f64 * rule.per_call_reserved_usage;
            rule.reserved_hours = rule.reserved_visits as f64 * rule.per_call_reserved_hours;
            rule.provider_id = Some(pool.provider_id.clone());
            rule.plan_id = Some(pool.plan_id.clone());
            rule.account_claim = format!(
                "{} / {} / {} owned account{} / {}",
                pool.provider_id,
                pool.plan_id,
                pool.subscription_count,
                if pool.subscription_count == 1 {
                    ""
                } else {
                    "s"
                },
                crate::agent_setup::binding_id_for(&policy.native_harness, &rows[policy.row_index],)
            );
            rule.fallback_policy = "If this binding is ineligible or its native window cannot fund a fresh hold, try the listed lower-effort route, then a listed qualified substitution for the same ready job; otherwise reduce authorized scope or defer and jointly replan. Never round-robin or choose from an open model menu.".to_owned();
            rule.calibration = "Benchmark evidence is provisional for this native harness. Native consumption, duration, and role outcomes remain uncalibrated until real-job telemetry supplies an interval and provenance.".to_owned();
            rule.fable = policy.fable;
            rule.message = String::new();
            if group.class.is_some() {
                pool.nominal_usage += rule.nominal_usage;
                pool.scheduled_hours += rule.nominal_hours;
                pool.reserved_usage += rule.reserved_usage;
                pool.reserved_hours += rule.reserved_hours;
            }
            if policy.fable {
                for limit in portfolio
                    .limits
                    .iter_mut()
                    .filter(|limit| limit.provider_id == pool.provider_id)
                {
                    limit.nominal_usage += rule.nominal_usage;
                    limit.reserved_usage += rule.reserved_usage;
                    limit.stress_usage = limit.reserved_usage;
                    limit.remaining_capacity =
                        (limit.weekly_capacity - limit.reserved_usage).max(0.0);
                }
            }
            portfolio.roles[group.role].allocated_hours += rule.nominal_hours;
            portfolio.roles[group.role].quality_utility += changes as f64 * factor * policy.utility;
        }
        if group.class.is_none() {
            let pool = &mut portfolio.pools[policy.pool];
            let headroom_usage = settings.orchestrator_headroom_usd.unwrap_or(0.0);
            let headroom_hours = settings.orchestrator_headroom_hours.unwrap_or(0.0);
            pool.reserved_usage += headroom_usage;
            pool.reserved_hours += headroom_hours;
            if policy.fable {
                for limit in portfolio
                    .limits
                    .iter_mut()
                    .filter(|limit| limit.provider_id == pool.provider_id)
                {
                    limit.reserved_usage += headroom_usage;
                }
            }
            portfolio.conductor = Some(ConductorAllocation {
                row_index: policy.row_index,
                binding_id: crate::agent_setup::binding_id_for(
                    &policy.native_harness,
                    &rows[policy.row_index],
                ),
                native_harness: policy.native_harness.clone(),
                provider_id: pool.provider_id.clone(),
                plan_id: pool.plan_id.clone(),
                attempt_limit: 1,
                competence: policy.competence,
                utility: policy.utility,
                persistent: true,
                evidence_profile: "Intelligence Index".to_owned(),
                evidence_level: policy.evidence_level.clone(),
                capability_sensitivity: Some(CapabilityRange {
                    nominal: policy.utility,
                    lower: policy.lower_utility,
                    upper: policy.upper_utility,
                }),
                usage_forecast: None,
                headroom: ConductorHeadroom {
                    usd: settings.orchestrator_headroom_usd,
                    hours: settings.orchestrator_headroom_hours,
                },
                per_call_expected_usage: policy.cycle.nominal_usage,
                per_call_expected_hours: policy.cycle.nominal_seconds / 3600.0,
                per_call_reserved_usage: policy.cycle.reserved_usage,
                per_call_reserved_hours: policy.cycle.reserved_seconds / 3600.0,
                cost_basis: rows[policy.row_index]
                    .orchestrator_cost_basis
                    .clone()
                    .unwrap_or_default(),
                time_basis: rows[policy.row_index]
                    .orchestrator_time_basis
                    .clone()
                    .unwrap_or_default(),
                classes: WorkClass::ALL
                    .into_iter()
                    .map(|class| ConductorClassDemand {
                        class,
                        changes: portfolio.dispatch.classes[class.index()].admitted_changes,
                        resource_factor: class.resource_factor(),
                        visits: 0,
                    })
                    .collect(),
            });
        }
    }
    adaptive_routes(&mut portfolio, &candidates, rows);
    for pool in &mut portfolio.pools {
        pool.stress_usage = pool.reserved_usage;
        pool.remaining_capacity = (pool.weekly_capacity - pool.nominal_usage).max(0.0);
        pool.remaining_reserved_capacity = (pool.weekly_capacity - pool.reserved_usage).max(0.0);
        pool.remaining_reserved_hours = (pool.available_hours - pool.reserved_hours).max(0.0);
        pool.unused_hours = (pool.available_hours - pool.scheduled_hours).max(0.0);
        pool.utilization_pct = if pool.weekly_capacity > 0.0 {
            100.0 * pool.reserved_usage / pool.weekly_capacity
        } else {
            0.0
        };
        portfolio.total_scheduled_hours += pool.scheduled_hours;
    }
    portfolio.total_agent_hours = portfolio.total_scheduled_hours;
    fill_math_audit(&mut portfolio);
    portfolio.message = format!(
        "{} of {} complete changes admitted; {} deferred. Reserved dependency makespan {:.2} of {:.2} hours. {}",
        best_count,
        forecast,
        forecast - best_count,
        portfolio.dispatch.reserved_makespan_hours,
        settings.agent_hours,
        portfolio.dispatch.solver.message
    );
    refresh_rate_usage(&mut portfolio, rows, settings);
    Some(portfolio)
}

fn fill_math_audit(portfolio: &mut Portfolio) {
    let share = |amount: f64, total: f64| if total > 0.0 { amount / total } else { 0.0 };
    for pool in &portfolio.pools {
        for role in &portfolio.roles {
            if role.seat == Seat::Orchestrator {
                continue;
            }
            let rules: Vec<_> = role
                .rules
                .iter()
                .filter(|rule| rule.provider_id.as_deref() == Some(&pool.provider_id))
                .collect();
            if rules.is_empty() {
                continue;
            }
            let mut row_indices: Vec<_> = rules.iter().filter_map(|rule| rule.row_index).collect();
            row_indices.sort_unstable();
            row_indices.dedup();
            let expected_usage = rules.iter().map(|rule| rule.nominal_usage).sum();
            let reserved_usage = rules.iter().map(|rule| rule.reserved_usage).sum();
            portfolio.math_audit.role_accounts.push(RoleAccountUsage {
                seat: role.seat,
                provider_id: pool.provider_id.clone(),
                row_indices,
                expected_usage,
                reserved_usage,
                expected_hours: rules.iter().map(|rule| rule.nominal_hours).sum(),
                reserved_hours: rules.iter().map(|rule| rule.reserved_hours).sum(),
                expected_account_share: share(expected_usage, pool.nominal_usage),
                reserved_account_share: share(reserved_usage, pool.reserved_usage),
            });
        }
    }
    if let Some(conductor) = &portfolio.conductor
        && let Some(pool) = portfolio.pools.iter().find(|pool| {
            pool.provider_id == conductor.provider_id && pool.plan_id == conductor.plan_id
        })
    {
        let reserved_usage = conductor.headroom.usd.unwrap_or(0.0);
        let reserved_hours = conductor.headroom.hours.unwrap_or(0.0);
        portfolio.math_audit.role_accounts.push(RoleAccountUsage {
            seat: Seat::Orchestrator,
            provider_id: conductor.provider_id.clone(),
            row_indices: vec![conductor.row_index],
            expected_usage: 0.0,
            reserved_usage,
            expected_hours: 0.0,
            reserved_hours,
            expected_account_share: 0.0,
            reserved_account_share: share(reserved_usage, pool.reserved_usage),
        });
    }
    let orchestrator: Vec<_> = portfolio
        .math_audit
        .role_accounts
        .iter()
        .filter(|account| account.seat == Seat::Orchestrator)
        .collect();
    portfolio.math_audit.orchestrator_expected_spend_share = share(
        orchestrator
            .iter()
            .map(|account| account.expected_usage)
            .sum(),
        portfolio.pools.iter().map(|pool| pool.nominal_usage).sum(),
    );
    portfolio.math_audit.orchestrator_reserved_spend_share = share(
        orchestrator
            .iter()
            .map(|account| account.reserved_usage)
            .sum(),
        portfolio.pools.iter().map(|pool| pool.reserved_usage).sum(),
    );
}

fn conditional_routes<'a>(
    policies: &'a [Policy],
    primary: &Policy,
    pool: usize,
    research_class: Option<WorkClass>,
) -> Vec<&'a Policy> {
    let qualified: Vec<_> = policies
        .iter()
        .filter(|policy| policy.pool == pool && policy.research_class == research_class)
        .collect();
    let frontier: Vec<_> = qualified
        .iter()
        .copied()
        .filter(|policy| {
            (policy.row_index != primary.row_index || policy.attempt_limit != primary.attempt_limit)
                && !qualified.iter().any(|other| {
                    (!other.fable || policy.fable)
                        && other.utility >= policy.utility
                        && other.competence >= policy.competence
                        && other.cycle.nominal_seconds <= policy.cycle.nominal_seconds
                        && other.cycle.nominal_usage <= policy.cycle.nominal_usage
                        && other.cycle.reserved_usage <= policy.cycle.reserved_usage
                        && other.cycle.reserved_seconds <= policy.cycle.reserved_seconds
                        && (other.utility > policy.utility
                            || other.competence > policy.competence
                            || other.cycle.nominal_seconds < policy.cycle.nominal_seconds
                            || other.cycle.nominal_usage < policy.cycle.nominal_usage
                            || other.cycle.reserved_usage < policy.cycle.reserved_usage
                            || other.cycle.reserved_seconds < policy.cycle.reserved_seconds)
                })
        })
        .collect();
    let mut selected = Vec::new();
    for criterion in 0..5 {
        let best = frontier.iter().copied().max_by(|left, right| {
            let value = |policy: &Policy| match criterion {
                0 => policy.utility,
                1 => policy.competence,
                2 => -policy.cycle.nominal_seconds,
                3 => -policy.cycle.nominal_usage,
                _ => -policy.cycle.reserved_usage,
            };
            value(left)
                .total_cmp(&value(right))
                .then_with(|| right.row_index.cmp(&left.row_index))
                .then_with(|| right.attempt_limit.cmp(&left.attempt_limit))
        });
        if let Some(policy) = best
            && !selected.iter().any(|other: &&Policy| {
                other.row_index == policy.row_index && other.attempt_limit == policy.attempt_limit
            })
        {
            selected.push(policy);
        }
    }
    let mut fastest = frontier;
    fastest.sort_by(|left, right| {
        left.cycle
            .nominal_seconds
            .total_cmp(&right.cycle.nominal_seconds)
            .then_with(|| right.utility.total_cmp(&left.utility))
            .then_with(|| left.row_index.cmp(&right.row_index))
            .then_with(|| left.attempt_limit.cmp(&right.attempt_limit))
    });
    for policy in fastest {
        if selected.len() == 6 {
            break;
        }
        if !selected.iter().any(|other| {
            other.row_index == policy.row_index && other.attempt_limit == policy.attempt_limit
        }) {
            selected.push(policy);
        }
    }
    selected
}
fn effort_level(row: &Row) -> Option<usize> {
    [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
    ]
    .iter()
    .position(|effort| Some(*effort) == row.effort.as_deref())
}
fn adaptive_routes(portfolio: &mut Portfolio, candidates: &[Vec<Policy>], rows: &[Row]) {
    for (role_index, role) in portfolio.roles.iter_mut().enumerate() {
        if role.seat == Seat::Orchestrator {
            continue;
        }
        for rule in &mut role.rules {
            let research_class = (role.seat == Seat::NetResearch).then_some(rule.class);
            let advantage: Vec<Vec<f64>> = candidates
                .iter()
                .map(|policies| {
                    (0..portfolio.pools.len())
                        .map(|pool| {
                            let own = policies
                                .iter()
                                .filter(|policy| {
                                    (policy.research_class.is_none()
                                        || policy.research_class == Some(rule.class))
                                        && policy.pool == pool
                                })
                                .map(|policy| policy.utility)
                                .fold(0.0, f64::max);
                            let other = policies
                                .iter()
                                .filter(|policy| {
                                    (policy.research_class.is_none()
                                        || policy.research_class == Some(rule.class))
                                        && policy.pool != pool
                                })
                                .map(|policy| policy.utility)
                                .fold(0.0, f64::max);
                            own - other
                        })
                        .collect()
                })
                .collect();
            let Some(index) = rule.row_index else {
                continue;
            };
            let Some(primary) = candidates[role_index].iter().find(|policy| {
                policy.row_index == index
                    && policy.attempt_limit == rule.attempt_limit
                    && policy.research_class == research_class
                    && rule.provider_id.as_deref()
                        == Some(portfolio.pools[policy.pool].provider_id.as_str())
                    && (role.seat != Seat::NetResearch
                        || rule.research_candidates.iter().any(|candidate| {
                            candidate.primary
                                && candidate.native_harness == policy.native_harness
                                && candidate.row_index == policy.row_index
                        }))
            }) else {
                continue;
            };
            let quality = primary.utility;
            let route = |policy: &Policy, purpose: &str| {
                let pool = &portfolio.pools[policy.pool];
                AdaptiveRoute {
                    row_index: policy.row_index,
                    binding_id: crate::agent_setup::binding_id_for(
                        &policy.native_harness,
                        &rows[policy.row_index],
                    ),
                    provider_id: pool.provider_id.clone(),
                    plan_id: pool.plan_id.clone(),
                    attempt_limit: policy.attempt_limit,
                    competence: policy.competence,
                    quality: policy.utility,
                    per_call_reserved_usage: policy.cycle.reserved_usage
                        * rule.class.resource_factor(),
                    per_call_reserved_hours: policy.cycle.reserved_seconds
                        * rule.class.resource_factor()
                        / 3600.0,
                    per_call_expected_usage: policy.cycle.nominal_usage
                        * rule.class.resource_factor(),
                    per_call_expected_hours: policy.cycle.nominal_seconds
                        * rule.class.resource_factor()
                        / 3600.0,
                    regret: (quality - policy.utility).max(0.0),
                    opportunity_cost: (advantage
                        .iter()
                        .map(|values| values[policy.pool])
                        .fold(f64::NEG_INFINITY, f64::max)
                        - advantage[role_index][policy.pool])
                        .max(0.0),
                    purpose: purpose.to_owned(),
                }
            };
            let preferred = |left: &&Policy, right: &&Policy| {
                (left.utility).total_cmp(&(right.utility)).then_with(|| {
                    right
                        .cycle
                        .reserved_usage
                        .total_cmp(&left.cycle.reserved_usage)
                })
            };
            rule.lower_effort = candidates[role_index]
                .iter()
                .filter(|policy| {
                    policy.pool == primary.pool
                        && policy.research_class == primary.research_class
                        && rows[policy.row_index].model == rows[index].model
                        && rows[policy.row_index].harness == rows[index].harness
                        && effort_level(&rows[policy.row_index])
                            .zip(effort_level(&rows[index]))
                            .is_some_and(|(lower, upper)| lower < upper)
                        && policy.cycle.reserved_usage < primary.cycle.reserved_usage
                })
                .max_by(preferred)
                .map(|policy| route(policy, "Lower effort for newly admitted work when quota pace is tight; requires fresh real-unit holds."));
            rule.within_provider_alternatives = conditional_routes(
                &candidates[role_index],
                primary,
                primary.pool,
                primary.research_class,
            )
                .into_iter()
                .map(|policy| route(policy, "Same-account alternative for distinct work; select from utility, latency and live quota after fresh holds."))
                .collect();
            rule.surplus_alternatives = (0..portfolio.pools.len())
                .filter(|pool| *pool != primary.pool)
                .flat_map(|pool| {
                    conditional_routes(
                        &candidates[role_index],
                        primary,
                        pool,
                        primary.research_class,
                    )
                        .into_iter()
                        .map(|policy| route(policy, "Competitive account for distinct useful surplus work; never duplicate an assigned job."))
                })
                .collect();
        }
    }
}
