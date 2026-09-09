use crate::engine::{self, DispatchCycle};
use crate::settings::Settings;
use crate::solver::{self, Choice, Outcome, Problem};
use crate::subscriptions;
use crate::types::{Row, Seat};
use serde::{Deserialize, Serialize};

const EXPECTED_VISITS: [f64; 7] = [1.0, 0.25, 1.25, 2.0, 1.25, 1.0, 0.0];
const RESERVED_VISITS: [usize; 7] = [1, 1, 2, 2, 2, 1, 0];
const SEARCH_NODES: usize = 1024;

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
    pub dependencies: &'static [usize],
    pub condition: &'static str,
}

pub const STAGES: &[WorkflowStage] = &[
    WorkflowStage {
        id: 0,
        seat: Seat::Orchestrator,
        visit: 1,
        dependencies: &[],
        condition: "Admitted change",
    },
    WorkflowStage {
        id: 1,
        seat: Seat::Comprehension,
        visit: 1,
        dependencies: &[0],
        condition: "Brief accepted",
    },
    WorkflowStage {
        id: 2,
        seat: Seat::Implementer,
        visit: 1,
        dependencies: &[1],
        condition: "Context established",
    },
    WorkflowStage {
        id: 3,
        seat: Seat::Reviewer,
        visit: 1,
        dependencies: &[2],
        condition: "Implementation produced immutable output; skip if implementation blocked",
    },
    WorkflowStage {
        id: 4,
        seat: Seat::Sanity,
        visit: 1,
        dependencies: &[2],
        condition: "Implementation produced immutable output; skip if implementation blocked",
    },
    WorkflowStage {
        id: 5,
        seat: Seat::Debugger,
        visit: 1,
        dependencies: &[3, 4],
        condition: "Implementation blocked or either initial check rejected; otherwise skip",
    },
    WorkflowStage {
        id: 6,
        seat: Seat::Reviewer,
        visit: 2,
        dependencies: &[5],
        condition: "Repair produced immutable output; otherwise skip",
    },
    WorkflowStage {
        id: 7,
        seat: Seat::Sanity,
        visit: 2,
        dependencies: &[5],
        condition: "Repair produced immutable output; otherwise skip",
    },
    WorkflowStage {
        id: 8,
        seat: Seat::Orchestrator,
        visit: 2,
        dependencies: &[6, 7],
        condition: "Accept checked output or close with bounded deferral",
    },
];

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
    #[serde(default)]
    pub native_admission_changes: usize,
    pub admitted_changes: usize,
    pub deferred_changes: usize,
    pub cadence_hours: f64,
    pub repair_incidence: f64,
    pub classes: Vec<WorkClassDemand>,
    pub solver: SolverReport,
    pub executable: bool,
    pub admitted_sequence: Vec<WorkClass>,
    pub reserved_makespan_hours: f64,
    pub schedule: Vec<ScheduledVisit>,
}

fn policy_version() -> String {
    format!("aitierlist-{}-team-policy-v1", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkClassDemand {
    pub class: WorkClass,
    pub condition: String,
    pub resource_factor: f64,
    pub forecast_changes: usize,
    pub admitted_changes: usize,
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
    pub non_hallucination: f64,
    pub non_hallucination_weight: f64,
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
    non_hallucination: f64,
    lcr: f64,
    hle: Option<f64>,
    expected_usd: Option<f64>,
    decode_hours: Option<f64>,
}

fn assess_research(row: &Row, weights: [f64; 4]) -> Option<ResearchAssessment> {
    let bounded =
        |value: Option<f64>| value.filter(|value| value.is_finite() && (0.0..=1.0).contains(value));
    let accuracy = bounded(row.omniscience_accuracy)?;
    let non_hallucination = 1.0 - bounded(row.halluc)?;
    let lcr = bounded(row.lcr)?;
    let hle = if weights[3] > 0.0 {
        Some(bounded(row.hle)?)
    } else {
        bounded(row.hle)
    };
    let direct_metric = |benchmark| {
        row.task_metrics.iter().find(|metric| {
            metric.benchmark == benchmark
                && metric
                    .time_basis
                    .starts_with("Canonical output decode estimate")
                && metric
                    .cost_basis
                    .starts_with("Canonical token-price estimate")
        })
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
        score: weights[0] * accuracy
            + weights[1] * non_hallucination
            + weights[2] * lcr
            + weights[3] * hle.unwrap_or(0.0),
        accuracy,
        non_hallucination,
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
}

struct Group {
    role: usize,
    class: Option<WorkClass>,
    policies: Vec<Policy>,
}

#[derive(Default)]
struct AccountCalendar {
    intervals: Vec<(f64, f64)>,
    usage: f64,
    fable_usage: f64,
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
    pub expected_visits: usize,
    pub reserved_visits: usize,
    pub expected_usage: f64,
    pub expected_hours: f64,
    pub reserved_usage: f64,
    pub reserved_hours: f64,
    pub per_call_expected_usage: f64,
    pub per_call_expected_hours: f64,
    pub per_call_reserved_usage: f64,
    pub per_call_reserved_hours: f64,
    pub cost_basis: String,
    pub time_basis: String,
    pub classes: Vec<ConductorClassDemand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConductorClassDemand {
    pub class: WorkClass,
    pub changes: usize,
    pub resource_factor: f64,
    pub visits: usize,
}

struct Schedule {
    visits: Vec<ScheduledVisit>,
    makespan: f64,
}

struct Search {
    groups: Vec<Group>,
    outcome: Outcome<Schedule>,
    count: usize,
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

fn sequence(total: usize) -> Vec<WorkClass> {
    let weights = [0.50, 0.30, 0.15, 0.05];
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

fn prune(mut policies: Vec<Policy>, rows: &[Row]) -> Vec<Policy> {
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
            !policies.iter().enumerate().any(|(other_index, other)| {
                other_index != index
                    && other.pool == policy.pool
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
    hours: f64,
) -> Option<Schedule> {
    let mut lookup = [[None; 6]; 4];
    let mut orchestrator = None;
    for (index, group) in groups.iter().enumerate() {
        if let Some(class) = group.class {
            lookup[class.index()][group.role] = Some(index);
        } else {
            orchestrator = Some(index);
        }
    }
    let mut policies = Vec::new();
    for class in sequence {
        for stage in STAGES {
            let role = Seat::ALL.iter().position(|seat| *seat == stage.seat)?;
            let group = if stage.seat == Seat::Orchestrator {
                orchestrator?
            } else {
                lookup[class.index()][role]?
            };
            policies.push(&groups[group].policies[choices[group]]);
        }
    }
    let mut finishes = vec![0.0_f64; policies.len()];
    let mut calendars: Vec<Vec<AccountCalendar>> = (0..pools.len()).map(|_| Vec::new()).collect();
    let mut visits = Vec::with_capacity(policies.len());
    let mut makespan = 0.0_f64;
    for (index, policy) in policies.iter().enumerate() {
        let stage = &STAGES[index % STAGES.len()];
        let base = index / STAGES.len() * STAGES.len();
        let class = sequence[index / STAGES.len()];
        let duration = policy.cycle.reserved_seconds * class.resource_factor() / 3600.0;
        let dependency_start = stage
            .dependencies
            .iter()
            .map(|dependency| finishes[base + dependency])
            .fold(0.0, f64::max);
        let usage = policy.cycle.reserved_usage * class.resource_factor();
        if calendars[policy.pool].is_empty() {
            calendars[policy.pool].push(AccountCalendar::default());
        }
        let existing = calendars[policy.pool].len();
        let candidates = if stage.seat == Seat::Orchestrator {
            1
        } else if existing < pools[policy.pool].subscription_count {
            existing + 1
        } else {
            existing
        };
        let (account_ordinal, start, finish) = (0..candidates)
            .filter(|account| {
                let account_usage = calendars[policy.pool]
                    .get(*account)
                    .map(|calendar| calendar.usage)
                    .unwrap_or(0.0);
                let account_fable_usage = calendars[policy.pool]
                    .get(*account)
                    .map(|calendar| calendar.fable_usage)
                    .unwrap_or(0.0);
                account_usage + usage <= pools[policy.pool].per_account_weekly_capacity + 1e-8
                    && (!policy.fable
                        || account_fable_usage + usage
                            <= pools[policy.pool].per_account_weekly_capacity * 0.5 + 1e-8)
            })
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
        calendar.usage += usage;
        if policy.fable {
            calendar.fable_usage += usage;
        }
        visits.push(ScheduledVisit {
            change_index: index / STAGES.len(),
            class,
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
    hours: f64,
    limit: usize,
) -> Search {
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
        policies: candidates[orchestrator_role].clone(),
    });
    for class in WorkClass::ALL {
        if counts[class.index()] > 0 {
            for (role, policies) in candidates.iter().enumerate() {
                if role == orchestrator_role || Seat::ALL[role] == Seat::NetResearch {
                    continue;
                }
                groups.push(Group {
                    role,
                    class: Some(class),
                    policies: policies.clone(),
                });
            }
        }
    }
    let mut capacities = Vec::new();
    for pool in pools {
        capacities.extend([pool.weekly_capacity, pool.available_hours]);
    }
    let fable_pool = pools
        .iter()
        .position(|pool| pool.provider_id == "anthropic" && pool.plan_id != "claude-pro");
    let fable_resource = fable_pool.map(|pool| {
        let index = capacities.len();
        capacities.push(pools[pool].weekly_capacity * 0.5);
        index
    });
    let path_resource = capacities.len();
    capacities.extend([hours; 8]);
    let choices = groups
        .iter()
        .map(|group| {
            let demands: Vec<_> = match group.class {
                Some(class) => vec![(counts[class.index()] as f64, class.resource_factor())],
                None => WorkClass::ALL
                    .into_iter()
                    .map(|class| (counts[class.index()] as f64, class.resource_factor()))
                    .collect(),
            };
            let visits = if group.role == orchestrator_role {
                2.0
            } else {
                EXPECTED_VISITS[group.role]
            };
            let expected_factor: f64 = demands
                .iter()
                .map(|(changes, factor)| changes * factor * visits)
                .sum();
            let reserved_factor: f64 = demands
                .iter()
                .map(|(changes, factor)| changes * factor * RESERVED_VISITS[group.role] as f64)
                .sum();
            group
                .policies
                .iter()
                .map(|policy| {
                    let mut resources = vec![0.0; capacities.len()];
                    resources[policy.pool * 2] = reserved_factor * policy.cycle.reserved_usage;
                    resources[policy.pool * 2 + 1] =
                        reserved_factor * policy.cycle.reserved_seconds / 3600.0;
                    if policy.fable
                        && let Some(index) = fable_resource
                    {
                        resources[index] = resources[policy.pool * 2];
                    }
                    for (class, changes) in WorkClass::ALL.into_iter().zip(counts) {
                        if group.class.is_some_and(|selected| selected != class) || changes == 0 {
                            continue;
                        }
                        let path = path_resource + class.index() * 2;
                        let factor = class.resource_factor();
                        resources[path] += [1.0, 1.0, 2.0, 2.0, 0.0, 1.0][group.role]
                            * factor
                            * policy.cycle.reserved_seconds
                            / 3600.0;
                        resources[path + 1] += [1.0, 1.0, 0.0, 2.0, 2.0, 1.0][group.role]
                            * factor
                            * policy.cycle.reserved_seconds
                            / 3600.0;
                    }
                    Choice {
                        quality: expected_factor * policy.utility,
                        nominal_time: expected_factor * policy.cycle.nominal_seconds / 3600.0,
                        nominal_cost: expected_factor * policy.cycle.nominal_usage,
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
    let outcome = solver::solve(&problem, limit, |choices| {
        schedule(&groups, choices, &sequence[..count], pools, hours)
    });
    Search {
        groups,
        outcome,
        count,
    }
}

fn seed_class_recommendations(
    portfolio: &mut Portfolio,
    candidates: &[Vec<Policy>],
    rows: &[Row],
    hours: f64,
) {
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
                    let reserve = policy.cycle.reserved_usage * factor;
                    reserve <= pool.per_account_weekly_capacity
                        && policy.cycle.reserved_seconds * factor / 3600.0 <= hours
                        && (!policy.fable || reserve <= pool.per_account_weekly_capacity * 0.5)
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
                rule.message = "No measured route can serve one visit for this class within a single owned account's declared proxy allowance and working window.".to_owned();
                continue;
            };
            let pool = &portfolio.pools[policy.pool];
            rule.row_index = Some(policy.row_index);
            rule.recommendation_only = true;
            rule.attempt_limit = policy.attempt_limit;
            rule.competence = Some(policy.competence);
            rule.utility = policy.utility;
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
                crate::agent_setup::binding_id(&rows[policy.row_index])
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
                        non_hallucination: assessment.non_hallucination,
                        non_hallucination_weight: weights[1],
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
                scope: "Standard research weights: Omniscience accuracy 0.30, non-hallucination 0.30, AA-LCR 0.40. This analytical tier comparison has no retry-throughput or coding prerequisite.".to_owned(),
            }
        })
        .collect()
}

pub fn allocate(rows: &[Row], settings: &Settings) -> Option<Portfolio> {
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
        settings
            .vendor_overrides
            .get(provider.id)
            .copied()
            .flatten()
            .unwrap_or(plan.monthly_allowance_low * 12.0 / 52.0)
    };
    let forecast = (settings.agent_hours * settings.orchestrators as f64 / 4.0).ceil() as usize;
    let sequence = sequence(forecast);
    let mut forecast_counts = [0_usize; 4];
    for class in &sequence {
        forecast_counts[class.index()] += 1;
    }
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
                fable: false, failure_action: failure_action(seat).to_owned(), message: if seat == Seat::NetResearch { "On demand; exact research tools, source scope, role evidence, billing, and native holds must qualify for the ready job." } else { "No funded static assignment is available for this workflow." }.to_owned(),
            }).collect(),
        }).collect(),
        pools: selected.iter().map(|(provider, plan)| PoolUsage {
            provider_id: provider.id.to_owned(), subscription_count: settings.subscription_count(provider.id), per_account_weekly_capacity: capacity(provider, plan), provider_name: provider.name.to_owned(), plan_id: plan.id.to_owned(), plan_name: plan.name.to_owned(), monthly_price: plan.monthly_price * settings.subscription_count(provider.id) as f64, price_is_estimate: plan.price_is_estimate,
            weekly_capacity: capacity(provider, plan) * settings.subscription_count(provider.id) as f64, nominal_usage: 0.0, stress_usage: 0.0, remaining_capacity: capacity(provider, plan) * settings.subscription_count(provider.id) as f64, utilization_pct: 0.0,
            available_hours: settings.agent_hours * settings.subscription_count(provider.id) as f64, scheduled_hours: 0.0, unused_hours: settings.agent_hours * settings.subscription_count(provider.id) as f64, reserved_usage: 0.0, reserved_hours: 0.0, remaining_reserved_capacity: capacity(provider, plan) * settings.subscription_count(provider.id) as f64, remaining_reserved_hours: settings.agent_hours * settings.subscription_count(provider.id) as f64,
            unit: "estimated API-equivalent USD".to_owned(), source_url: plan.source_url.to_owned(), allowance_basis: if settings.vendor_overrides.get(provider.id).copied().flatten().is_some() { format!("User-defined weekly API-equivalent allowance per subscription × {} owned subscriptions", settings.subscription_count(provider.id)) } else { format!("{} Owned allowance units: {}.", plan.allowance_basis, settings.subscription_count(provider.id)) },
        }).collect(),
        limits: selected.iter().filter(|(provider, plan)| provider.id == "anthropic" && plan.id != "claude-pro").map(|(provider, plan)| PoolLimit {
            provider_id: provider.id.to_owned(), name: "Fable: 50% of the aggregate Claude account pools".to_owned(), weekly_capacity: capacity(provider, plan) * settings.subscription_count(provider.id) as f64 * 0.5, nominal_usage: 0.0, stress_usage: 0.0, reserved_usage: 0.0, remaining_capacity: capacity(provider, plan) * settings.subscription_count(provider.id) as f64 * 0.5,
        }).collect(),
        assumptions: vec![
            "The proxy demand volume is ceil(elapsed weekly hours * concurrent project orchestrators / 4). It is a capacity scenario, not an authorized backlog or native admission. Orchestrators share every subscription allowance; the count does not multiply quota or approved worker concurrency. The 50/30/15/5% template mix and 0.25/1/2/4 proxy factors are declared conventions, not measured workload averages.".to_owned(),
            "Each owned subscription contributes one modeled allowance and one account calendar within the same elapsed working horizon. Forecast account ordinals are scheduling slots, not native account identities, authentication, meters, or authorization. The fixed conductor remains on the first account slot.".to_owned(),
            "Select workflow templates from consequence, uncertainty, coupling, reversibility, evidence need, tool risk, correlation, and deadline risk. File count is load information only and never determines the workflow.".to_owned(),
            "The full forecast uses largest-remainder class counts with canonical ties. A fixed deficit-ordered sequence supplies nested admission prefixes; reduced demand never re-apportions.".to_owned(),
            "The API-equivalent proxy reserves Orchestrator 2, Comprehension 1, Implementer 1, Debugger 1, Reviewer 2, and Sanity 2 visits per proxy job, with 25% expected repair incidence. Native tasks expand from their risk vectors and add Net Research only when evidence is required.".to_owned(),
            "The Orchestrator uses one persistent model and effort for both visits across the whole plan. Its resource load is two Intelligence Index benchmark-task-equivalent units per class-weighted admitted change; every real native call is also charged once in the runtime ledger. Each worker visit uses one model and effort with 1–3 same-model attempts.".to_owned(),
            "Multi-criteria role utility uses each skill component once: capped completion for reference-workload evidence, static values for other skills. It is not real-job success probability. Class factors scale demand and resources, not calibrated difficulty.".to_owned(),
            "The proxy calendar places its declared stage sequence in earliest account gaps to estimate API-equivalent capacity. Native scheduling instead uses authorized ready task IDs, risk-selected DAGs, immutable artifact dependencies, current reset windows, and deadlines.".to_owned(),
            "The solver optimizes only assignments feasible under this declared scheduler. Modeled allowances and full-cap buffers do not guarantee real vendor quota or runtime; unknown actual usage requires pause and reconciliation.".to_owned(),
        ],
        message: String::new(),
        math_audit: MathAudit {
            objective: "Multi-criteria role utility uses each competence component once: capped completion replaces reference-workload evidence, while other skill components remain static. Competence floors apply separately; this is not a calibrated probability of completing a real job.".to_owned(),
            coordination_proxy: "Orchestration is charged two class-weighted benchmark-task-equivalent units per admitted change using the selected model's Artificial Analysis Intelligence Index cost and canonical decode-time proxy. The Intelligence Index is selection evidence, not a measured probability of orchestration success; every actual native call is charged once in the runtime ledger.".to_owned(),
            ..MathAudit::default()
        },
        conductor: None,
        dispatch: DispatchPlan {
            policy_version: crate::team_policy::VERSION.to_owned(),
            forecast_changes: forecast, native_admission_changes: 0, admitted_changes: 0, deferred_changes: forecast, cadence_hours: 4.0, repair_incidence: 0.25,
            classes: WorkClass::ALL.into_iter().map(|class| WorkClassDemand { class, condition: class.condition().to_owned(), resource_factor: class.resource_factor(), forecast_changes: forecast_counts[class.index()], admitted_changes: 0 }).collect(),
            solver: SolverReport { status: "infeasible".to_owned(), proven_optimal: false, quality: 0.0, bound: None, relative_gap: None, nodes: 0, message: String::new() },
            executable: false, admitted_sequence: Vec::new(), reserved_makespan_hours: 0.0, schedule: Vec::new(),
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
                        non_hallucination: assessment.non_hallucination,
                        non_hallucination_weight: weights[1],
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
            let mut account_champions = std::collections::BTreeMap::new();
            for candidate in &candidates {
                account_champions
                    .entry((
                        candidate.provider_id.clone(),
                        candidate.native_harness.clone(),
                    ))
                    .or_insert_with(|| {
                        (
                            candidate.binding_id.clone(),
                            crate::aa::family_key("", &rows[candidate.row_index].model_key),
                        )
                    });
            }
            let frontier: Vec<_> = candidates
                .iter()
                .enumerate()
                .map(|(index, candidate)| {
                    let account = (
                        candidate.provider_id.clone(),
                        candidate.native_harness.clone(),
                    );
                    let Some((champion, family)) = account_champions.get(&account) else {
                        return false;
                    };
                    index == 0
                        || candidate.binding_id == *champion
                        || (crate::aa::family_key("", &rows[candidate.row_index].model_key)
                            == *family
                            && !candidates.iter().any(|other| {
                                let resources = other
                                    .expected_usd
                                    .zip(other.decode_hours)
                                    .zip(candidate.expected_usd.zip(candidate.decode_hours));
                                other.provider_id == candidate.provider_id
                                    && other.native_harness == candidate.native_harness
                                    && crate::aa::family_key("", &rows[other.row_index].model_key)
                                        == *family
                                    && resources.is_some_and(
                                        |(
                                            (other_usd, other_hours),
                                            (candidate_usd, candidate_hours),
                                        )| {
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
            rule.research_candidates = candidates
                .into_iter()
                .zip(frontier)
                .filter_map(|(candidate, keep)| keep.then_some(candidate))
                .collect();
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
                row.smart
            } else {
                engine::competence(row, seat)
            };
            let Some(competence) = competence.filter(|value| {
                value.is_finite() && (0.0..=1.0).contains(value) && *value >= floor && *value > 0.0
            }) else {
                continue;
            };
            if seat == Seat::Orchestrator {
                let Some(cycle) = engine::orchestrator_dispatch_cycle(row, settings) else {
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
                    utility: competence,
                    attempt_limit: 1,
                    cycle,
                    fable: subscriptions::is_fable(row) && row.vendor == "anthropic",
                });
                continue;
            }
            let Some(cycles) =
                cycles[usize::from(!matches!(seat, Seat::Implementer | Seat::Debugger))]
            else {
                continue;
            };
            for (index, cycle) in cycles.into_iter().enumerate() {
                if cycle.success > 0.0
                    && let Some(utility) = engine::dispatch_utility(row, seat, &cycle)
                {
                    candidates[role].push(Policy {
                        row_index,
                        native_harness: row.harness.clone(),
                        pool,
                        competence,
                        utility,
                        attempt_limit: index + 1,
                        cycle,
                        fable: subscriptions::is_fable(row) && row.vendor == "anthropic",
                    });
                }
            }
        }
    }
    let candidates: Vec<_> = candidates
        .into_iter()
        .map(|policies| prune(policies, rows))
        .collect();
    seed_class_recommendations(&mut portfolio, &candidates, rows, settings.agent_hours);
    let mut nodes = 0;
    let mut unresolved = false;
    let initial = search(
        forecast,
        &sequence,
        &candidates,
        &portfolio.pools,
        settings.agent_hours,
        256,
    );
    nodes += initial.outcome.nodes;
    let mut numerical_failure = initial.outcome.limitation;
    let mut best = if initial.outcome.solution.is_some() {
        Some(initial)
    } else {
        unresolved |= !initial.outcome.proven;
        None
    };
    if best.is_none() {
        let mut lower = 0;
        let mut upper = forecast;
        while lower + 1 < upper && nodes < SEARCH_NODES {
            let count = (lower + upper) / 2;
            let result = search(
                count,
                &sequence,
                &candidates,
                &portfolio.pools,
                settings.agent_hours,
                (SEARCH_NODES - nodes).min(256),
            );
            nodes += result.outcome.nodes;
            if let Some(reason) = result.outcome.limitation {
                numerical_failure.get_or_insert(reason);
            }
            if result.outcome.solution.is_some() {
                lower = count;
                best = Some(result);
            } else {
                unresolved |= !result.outcome.proven || result.outcome.limitation.is_some();
                upper = count;
            }
        }
        if lower + 1 < upper {
            unresolved = true;
        }
    }
    portfolio.dispatch.solver.nodes = nodes;
    let Some(best) = best else {
        portfolio.dispatch.solver.status = if numerical_failure.is_some() {
            "solver_failure"
        } else if unresolved {
            "search_limit"
        } else {
            "infeasible"
        }
        .to_owned();
        portfolio.message = if let Some(reason) = numerical_failure {
            format!(
                "The numerical relaxation stopped: {reason}. No complete feasible policy was found; infeasibility is not proved."
            )
        } else if unresolved {
            "Search limit reached without an executable complete-change plan; infeasibility is not proved.".to_owned()
        } else {
            "No complete forecast change can be funded with all required roles and the bounded dependency schedule.".to_owned()
        };
        portfolio.dispatch.solver.message = portfolio.message.clone();
        adaptive_routes(&mut portfolio, &candidates, rows);
        return Some(portfolio);
    };
    let solution = best.outcome.solution.expect("selected dispatch solution");
    let proven = best.outcome.proven && !unresolved;
    portfolio.dispatch.admitted_changes = best.count;
    portfolio.dispatch.deferred_changes = forecast - best.count;
    portfolio.dispatch.executable = true;
    portfolio.dispatch.admitted_sequence = sequence[..best.count].to_vec();
    portfolio.dispatch.reserved_makespan_hours = solution.payload.makespan;
    portfolio.dispatch.schedule = solution.payload.visits;
    for class in &sequence[..best.count] {
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
    portfolio.dispatch.solver.quality = solution.quality;
    portfolio.dispatch.solver.bound = best.outcome.bound;
    portfolio.dispatch.solver.relative_gap = best.outcome.bound.map(|bound| {
        ((bound - solution.quality).max(0.0) / solution.quality.max(f64::MIN_POSITIVE)).max(0.0)
    });
    portfolio.dispatch.solver.message = if proven {
        "Maximum complete-change admission and quality are optimal within the declared scheduler and numerical tolerances.".to_owned()
    } else if let Some(reason) = numerical_failure {
        format!(
            "A feasible complete-change plan is available. The numerical relaxation stopped: {reason}; optimality is not proved."
        )
    } else {
        "A feasible complete-change plan is available; the bounded search did not prove all admission or quality decisions optimal.".to_owned()
    };
    for (group, choice) in best.groups.iter().zip(solution.choices) {
        let policy = &group.policies[choice];
        let classes: Vec<_> = group
            .class
            .map_or_else(|| WorkClass::ALL.to_vec(), |class| vec![class]);
        for class in classes {
            let changes = portfolio.dispatch.classes[class.index()].admitted_changes;
            let visits = if group.class.is_none() {
                2
            } else {
                RESERVED_VISITS[group.role]
            };
            let expected = if group.class.is_none() {
                (changes * 2) as f64
            } else {
                changes as f64 * EXPECTED_VISITS[group.role]
            };
            let factor = class.resource_factor();
            let pool = &mut portfolio.pools[policy.pool];
            let seat = portfolio.roles[group.role].seat;
            let rule = &mut portfolio.roles[group.role].rules[class.index()];
            rule.row_index = Some(policy.row_index);
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
                crate::agent_setup::binding_id(&rows[policy.row_index])
            );
            rule.fallback_policy = "If this binding is ineligible or its native window cannot fund a fresh hold, try the listed lower-effort route, then a listed qualified substitution for the same ready job; otherwise reduce authorized scope or defer and jointly replan. Never round-robin or choose from an open model menu.".to_owned();
            rule.calibration = "Benchmark evidence is provisional for this native harness. Native consumption, duration, and role outcomes remain uncalibrated until real-job telemetry supplies an interval and provenance.".to_owned();
            rule.fable = policy.fable;
            rule.message = String::new();
            pool.nominal_usage += rule.nominal_usage;
            pool.scheduled_hours += rule.nominal_hours;
            pool.reserved_usage += rule.reserved_usage;
            pool.reserved_hours += rule.reserved_hours;
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
            portfolio.roles[group.role].quality_utility += expected * factor * policy.utility;
        }
        if group.class.is_none() {
            let rules = &portfolio.roles[group.role].rules;
            let pool = &portfolio.pools[policy.pool];
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
                expected_visits: best.count * 2,
                reserved_visits: best.count * 2,
                expected_usage: rules.iter().map(|rule| rule.nominal_usage).sum(),
                expected_hours: rules.iter().map(|rule| rule.nominal_hours).sum(),
                reserved_usage: rules.iter().map(|rule| rule.reserved_usage).sum(),
                reserved_hours: rules.iter().map(|rule| rule.reserved_hours).sum(),
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
                        visits: portfolio.dispatch.classes[class.index()].admitted_changes * 2,
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
        best.count,
        forecast,
        forecast - best.count,
        portfolio.dispatch.reserved_makespan_hours,
        settings.agent_hours,
        portfolio.dispatch.solver.message
    );
    Some(portfolio)
}

fn fill_math_audit(portfolio: &mut Portfolio) {
    let share = |amount: f64, total: f64| if total > 0.0 { amount / total } else { 0.0 };
    for pool in &portfolio.pools {
        for role in &portfolio.roles {
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
) -> Vec<&'a Policy> {
    let qualified: Vec<_> = policies
        .iter()
        .filter(|policy| policy.pool == pool)
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
    let advantage: Vec<Vec<f64>> = candidates
        .iter()
        .map(|policies| {
            (0..portfolio.pools.len())
                .map(|pool| {
                    let own = policies
                        .iter()
                        .filter(|policy| policy.pool == pool)
                        .map(|policy| policy.utility)
                        .fold(0.0, f64::max);
                    let other = policies
                        .iter()
                        .filter(|policy| policy.pool != pool)
                        .map(|policy| policy.utility)
                        .fold(0.0, f64::max);
                    own - other
                })
                .collect()
        })
        .collect();
    for (role_index, role) in portfolio.roles.iter_mut().enumerate() {
        if role.seat == Seat::Orchestrator {
            continue;
        }
        for rule in &mut role.rules {
            let Some(index) = rule.row_index else {
                continue;
            };
            let Some(primary) = candidates[role_index].iter().find(|policy| {
                policy.row_index == index && policy.attempt_limit == rule.attempt_limit
            }) else {
                continue;
            };
            let quality = primary.utility;
            let route = |policy: &Policy, purpose: &str| {
                let pool = &portfolio.pools[policy.pool];
                AdaptiveRoute {
                    row_index: policy.row_index,
                    binding_id: crate::agent_setup::binding_id(&rows[policy.row_index]),
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
                        && rows[policy.row_index].model == rows[index].model
                        && rows[policy.row_index].harness == rows[index].harness
                        && effort_level(&rows[policy.row_index])
                            .zip(effort_level(&rows[index]))
                            .is_some_and(|(lower, upper)| lower < upper)
                        && policy.cycle.reserved_usage < primary.cycle.reserved_usage
                })
                .max_by(preferred)
                .map(|policy| route(policy, "Lower effort for newly admitted work when quota pace is tight; requires fresh real-unit holds."));
            rule.within_provider_alternatives = conditional_routes(&candidates[role_index], primary, primary.pool)
                .into_iter()
                .map(|policy| route(policy, "Same-account alternative for distinct work; select from utility, latency and live quota after fresh holds."))
                .collect();
            rule.surplus_alternatives = (0..portfolio.pools.len())
                .filter(|pool| *pool != primary.pool)
                .flat_map(|pool| {
                    conditional_routes(&candidates[role_index], primary, pool)
                        .into_iter()
                        .map(|policy| route(policy, "Competitive account for distinct useful surplus work; never duplicate an assigned job."))
                })
                .collect();
        }
    }
}
