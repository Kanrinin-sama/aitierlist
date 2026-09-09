use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::agent_setup::{JobStatus, Ledger, NativeCoefficient, SetupProfile, profile_identity};
use crate::isolation::Route;
use crate::route_qualification::qualify;
use crate::team_policy::{Assignment, NativePlan, TaskRequest, VERSION, Visit, visits};
use crate::types::Seat;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchStatus {
    Exhaustive,
    FeasibleSearchLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DeferredTask {
    pub task_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DominantShare {
    pub project_id: String,
    pub share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ScheduledVisit {
    pub task_id: String,
    pub visit_id: String,
    pub start: String,
    pub end: String,
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NativeWindowPacing {
    pub account_id: String,
    pub window_id: String,
    pub unit: String,
    pub available: f64,
    pub planned_expected: f64,
    pub planned_reserve: f64,
    pub uncertainty_lower: f64,
    pub uncertainty_upper: f64,
    pub remaining_slack: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NativeHorizon {
    pub policy_version: String,
    pub profile_identity: String,
    pub ledger_revision: u64,
    pub horizon_id: String,
    pub plans: Vec<NativePlan>,
    pub deferred: Vec<DeferredTask>,
    pub dominant_shares: Vec<DominantShare>,
    pub scheduled_visits: Vec<ScheduledVisit>,
    pub window_pacing: Vec<NativeWindowPacing>,
    pub search_status: SearchStatus,
    pub explored_nodes: usize,
    pub feasible_gap: Option<f64>,
}

type Resource = (String, String, String);

#[derive(Clone)]
struct CandidatePlan {
    plan: NativePlan,
    usage: BTreeMap<Resource, f64>,
    account_seconds: BTreeMap<String, f64>,
    host_seconds: BTreeMap<String, f64>,
    exclusive_seconds: BTreeMap<String, f64>,
    human_seconds: f64,
    pilot_keys: Vec<String>,
    visit_seconds: Vec<f64>,
    seconds: f64,
    reset_at: Option<OffsetDateTime>,
}

#[derive(Clone, Default)]
struct Score {
    high_consequence: u64,
    deadline: u64,
    useful: u64,
    fairness_cost: f64,
    artifact_value: f64,
    resource_cost: f64,
    seconds: f64,
}

impl Score {
    fn compare(&self, other: &Self) -> Ordering {
        self.high_consequence
            .cmp(&other.high_consequence)
            .then_with(|| self.deadline.cmp(&other.deadline))
            .then_with(|| self.useful.cmp(&other.useful))
            .then_with(|| other.fairness_cost.total_cmp(&self.fairness_cost))
            .then_with(|| self.artifact_value.total_cmp(&other.artifact_value))
            .then_with(|| other.resource_cost.total_cmp(&self.resource_cost))
            .then_with(|| other.seconds.total_cmp(&self.seconds))
    }
}

struct Search<'a> {
    tasks: Vec<&'a TaskRequest>,
    candidates: Vec<Vec<CandidatePlan>>,
    capacity: BTreeMap<Resource, f64>,
    endowment: BTreeMap<Resource, f64>,
    account_seconds_capacity: f64,
    host_seconds_capacity: f64,
    horizon_seconds: f64,
    human_seconds_capacity: f64,
    active_windows: Vec<(OffsetDateTime, OffsetDateTime)>,
    existing_project_usage: BTreeMap<String, BTreeMap<Resource, f64>>,
    planning_at: OffsetDateTime,
    project_priorities: BTreeMap<String, u32>,
    project_weights: BTreeMap<String, f64>,
    node_limit: usize,
    nodes: usize,
    limited: bool,
    incumbent: Vec<Option<usize>>,
    incumbent_score: Score,
}

#[derive(Default)]
struct SearchState {
    usage: BTreeMap<Resource, f64>,
    account_seconds: BTreeMap<String, f64>,
    host_seconds: BTreeMap<String, f64>,
    exclusive_seconds: BTreeMap<String, f64>,
    human_seconds: f64,
    pilots: BTreeSet<String>,
    project_counts: BTreeMap<String, u32>,
}

pub fn allocate(
    tasks: &[TaskRequest],
    routes: &[Route],
    profile: &SetupProfile,
    ledger: &Ledger,
    running_conductor_binding_id: Option<&str>,
    horizon_id: &str,
    node_limit: usize,
) -> Result<NativeHorizon> {
    ensure!(!horizon_id.trim().is_empty(), "Horizon identity is empty");
    ensure!(node_limit > 0, "Search node limit must be positive");
    let frozen_conductor = running_conductor_binding(ledger)?;
    if let (Some(requested), Some(frozen)) =
        (running_conductor_binding_id, frozen_conductor.as_deref())
    {
        ensure!(
            requested == frozen,
            "Running conductor binding cannot change"
        );
    }
    let fixed_conductor = running_conductor_binding_id.or(frozen_conductor.as_deref());
    let conductors = conductor_bindings(routes, profile, fixed_conductor);
    ensure!(
        !conductors.is_empty(),
        "No qualified conductor binding is available"
    );

    let mut eligible = Vec::new();
    let mut deferred = Vec::new();
    for task in tasks {
        match task_eligibility(task, ledger) {
            Ok(()) => eligible.push(task),
            Err(error) => deferred.push(DeferredTask {
                task_id: task.task_id.clone(),
                reason: format!("{error:#}"),
            }),
        }
    }
    eligible.sort_by(|left, right| {
        right
            .effective_risk()
            .consequence
            .cmp(&left.effective_risk().consequence)
            .then_with(|| deadline_key(left).cmp(&deadline_key(right)))
            .then_with(|| project_priority(profile, right).cmp(&project_priority(profile, left)))
            .then_with(|| left.task_id.cmp(&right.task_id))
    });

    let replaceable: BTreeSet<_> = eligible.iter().map(|task| task.task_id.as_str()).collect();
    let capacity = match native_capacity(profile, ledger, &replaceable) {
        Ok(capacity) => capacity,
        Err(error) => {
            for task in &eligible {
                deferred.push(DeferredTask {
                    task_id: task.task_id.clone(),
                    reason: format!("Native capacity facts are unavailable: {error:#}"),
                });
            }
            return empty_horizon(profile, ledger, horizon_id, deferred);
        }
    };
    let active_windows = match active_windows(profile, ledger) {
        Ok(windows) => windows,
        Err(error) => {
            for task in &eligible {
                deferred.push(DeferredTask {
                    task_id: task.task_id.clone(),
                    reason: format!("Scheduling facts are unavailable: {error:#}"),
                });
            }
            return empty_horizon(profile, ledger, horizon_id, deferred);
        }
    };
    let horizon_seconds: f64 = active_windows
        .iter()
        .map(|(start, end)| (*end - *start).as_seconds_f64())
        .sum();
    let Some(human_seconds_capacity) = profile
        .workflow
        .human_available_seconds
        .as_ref()
        .and_then(|fact| fact.value)
    else {
        for task in &eligible {
            deferred.push(DeferredTask {
                task_id: task.task_id.clone(),
                reason: "Scheduling facts are unavailable: human capacity is unknown".to_owned(),
            });
        }
        return empty_horizon(profile, ledger, horizon_id, deferred);
    };
    let concurrency = profile
        .workflow
        .concurrency
        .value
        .context("Host concurrency is unknown")?;
    ensure!(
        horizon_seconds.is_finite()
            && horizon_seconds >= 0.0
            && concurrency > 0
            && human_seconds_capacity.is_finite()
            && human_seconds_capacity >= 0.0,
        "Invalid horizon capacity"
    );
    let mut best: Option<(String, Search)> = None;
    for conductor in conductors {
        let mut candidate_sets = Vec::new();
        let mut candidate_limited = false;
        for task in &eligible {
            let (plans, limited) =
                candidate_plans(task, routes, profile, ledger, &conductor, node_limit)?;
            candidate_sets.push(plans);
            candidate_limited |= limited;
        }
        let mut search = Search {
            tasks: eligible.clone(),
            candidates: candidate_sets,
            capacity: capacity.clone(),
            endowment: native_endowment(profile),
            account_seconds_capacity: horizon_seconds,
            host_seconds_capacity: horizon_seconds * f64::from(concurrency),
            horizon_seconds,
            human_seconds_capacity,
            active_windows: active_windows.clone(),
            existing_project_usage: existing_project_usage(ledger, &replaceable),
            planning_at: OffsetDateTime::parse(&ledger.updated_at, &Rfc3339)?,
            project_priorities: eligible
                .iter()
                .map(|task| (task.project_id.clone(), project_priority(profile, task)))
                .collect(),
            project_weights: eligible
                .iter()
                .map(|task| (task.project_id.clone(), project_weight(profile, task)))
                .collect(),
            node_limit,
            nodes: 0,
            limited: false,
            incumbent: vec![None; eligible.len()],
            incumbent_score: Score::default(),
        };
        let mut selection = vec![None; eligible.len()];
        let mut state = SearchState {
            pilots: unresolved_pilots(profile, ledger),
            ..SearchState::default()
        };
        search.walk(0, &mut selection, &mut state);
        search.limited |= candidate_limited;
        let replace = best.as_ref().is_none_or(|(_, current)| {
            search.incumbent_score.compare(&current.incumbent_score) == Ordering::Greater
        });
        if replace {
            best = Some((conductor, search));
        }
    }
    let (_, search) = best.context("No conductor search was performed")?;
    let mut plans = Vec::new();
    for (index, choice) in search.incumbent.iter().enumerate() {
        if let Some(choice) = choice {
            plans.push(search.candidates[index][*choice].plan.clone());
        } else {
            let reason = if search.candidates[index].is_empty() {
                "No exact qualified route bundle fits every mandatory and conditional visit"
            } else {
                "Not admitted within the current native-window, time, and fairness allocation"
            };
            deferred.push(DeferredTask {
                task_id: search.tasks[index].task_id.clone(),
                reason: reason.to_owned(),
            });
        }
    }
    let dominant_shares = dominant_shares(
        &plans,
        &search.endowment,
        &search.existing_project_usage,
        &search.project_weights,
    );
    let scheduled_visits = schedule(&plans, &eligible, profile, ledger, &active_windows)?;
    let window_pacing = window_pacing(&plans, &capacity);
    deferred.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    Ok(NativeHorizon {
        policy_version: VERSION.to_owned(),
        profile_identity: profile_identity(profile)?,
        ledger_revision: ledger.revision,
        horizon_id: horizon_id.to_owned(),
        plans,
        deferred,
        dominant_shares,
        scheduled_visits,
        window_pacing,
        search_status: if search.limited {
            SearchStatus::FeasibleSearchLimit
        } else {
            SearchStatus::Exhaustive
        },
        explored_nodes: search.nodes,
        feasible_gap: None,
    })
}

fn empty_horizon(
    profile: &SetupProfile,
    ledger: &Ledger,
    horizon_id: &str,
    mut deferred: Vec<DeferredTask>,
) -> Result<NativeHorizon> {
    deferred.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    Ok(NativeHorizon {
        policy_version: VERSION.to_owned(),
        profile_identity: profile_identity(profile)?,
        ledger_revision: ledger.revision,
        horizon_id: horizon_id.to_owned(),
        plans: Vec::new(),
        deferred,
        dominant_shares: Vec::new(),
        scheduled_visits: Vec::new(),
        window_pacing: Vec::new(),
        search_status: SearchStatus::Exhaustive,
        explored_nodes: 0,
        feasible_gap: None,
    })
}

fn running_conductor_binding(ledger: &Ledger) -> Result<Option<String>> {
    let bindings: BTreeSet<_> = ledger
        .jobs
        .iter()
        .filter(|job| job.status == JobStatus::Running && job.seat == Some(Seat::Orchestrator))
        .map(|job| job.binding_id.clone())
        .collect();
    ensure!(
        bindings.len() <= 1,
        "Running conductors use different bindings"
    );
    Ok(bindings.into_iter().next())
}

fn conductor_bindings(
    routes: &[Route],
    profile: &SetupProfile,
    fixed: Option<&str>,
) -> Vec<String> {
    let mut bindings: Vec<_> = routes
        .iter()
        .filter(|route| fixed.is_none_or(|binding| route.binding_id == binding))
        .filter(|route| route.authenticated)
        .filter(|route| {
            profile
                .bindings
                .iter()
                .find(|binding| binding.id == route.binding_id)
                .is_some_and(|binding| binding.subscription_funded(false))
        })
        .map(|route| route.binding_id.clone())
        .collect();
    bindings.sort();
    bindings.dedup();
    bindings
}

fn task_eligibility(task: &TaskRequest, ledger: &Ledger) -> Result<()> {
    ensure!(
        !task.task_id.is_empty() && !task.project_id.is_empty() && !task.idempotency_key.is_empty(),
        "Task identity is incomplete"
    );
    ensure!(task.authorized, "Task is not authorized");
    ensure!(task.ready, "Task is not ready");
    ensure!(task.timeout_seconds > 0, "Task timeout is unavailable");
    ensure!(
        task.artifact_value.is_finite() && task.artifact_value >= 0.0,
        "Artifact value is invalid"
    );
    ensure!(
        task.human_seconds.is_finite() && task.human_seconds >= 0.0,
        "Human capacity requirement is invalid"
    );
    let now = OffsetDateTime::parse(&ledger.updated_at, &Rfc3339)
        .context("Ledger planning timestamp is invalid")?;
    if let Some(ready_at) = &task.ready_at {
        ensure!(
            OffsetDateTime::parse(ready_at, &Rfc3339).context("Task ready time is invalid")? <= now,
            "Task has not reached its ready time"
        );
    }
    if let Some(deadline) = &task.deadline {
        ensure!(
            OffsetDateTime::parse(deadline, &Rfc3339).context("Task deadline is invalid")? > now,
            "Task deadline has expired"
        );
    }
    Ok(())
}

fn project_priority(profile: &SetupProfile, task: &TaskRequest) -> u32 {
    profile
        .project_policies
        .iter()
        .find(|policy| policy.project_id == task.project_id)
        .map_or(0, |policy| policy.priority)
}

fn project_weight(profile: &SetupProfile, task: &TaskRequest) -> f64 {
    profile
        .project_policies
        .iter()
        .find(|policy| policy.project_id == task.project_id)
        .map_or(1.0, |policy| policy.entitlement_weight)
}

fn policy_task(profile: &SetupProfile, task: &TaskRequest) -> TaskRequest {
    let mut task = task.clone();
    task.risk = task.effective_risk();
    if let Some(policy) = profile
        .project_policies
        .iter()
        .find(|policy| policy.project_id == task.project_id)
    {
        let floor = match policy.minimum_workflow {
            crate::portfolio::WorkClass::Focused => 0,
            crate::portfolio::WorkClass::Standard => 1,
            crate::portfolio::WorkClass::Complex => 2,
            crate::portfolio::WorkClass::Extensive => 3,
        };
        task.risk.uncertainty = task.risk.uncertainty.max(floor);
        for (seat, threshold) in &policy.role_thresholds {
            task.role_thresholds
                .entry(seat.clone())
                .and_modify(|value| *value = value.max(*threshold))
                .or_insert(*threshold);
        }
    }
    task
}

fn native_capacity(
    profile: &SetupProfile,
    ledger: &Ledger,
    replaceable: &BTreeSet<&str>,
) -> Result<BTreeMap<Resource, f64>> {
    let mut capacity = BTreeMap::new();
    for account in &ledger.accounts {
        for window in &account.windows {
            let Some(snapshot) = window.snapshot.value.as_ref() else {
                continue;
            };
            let Some(unit) = profile
                .accounts
                .iter()
                .find(|item| item.id == account.account_id)
                .and_then(|item| {
                    item.quota_windows
                        .iter()
                        .find(|item| item.id == window.window_id)
                })
                .and_then(|item| item.unit.value.as_ref())
            else {
                continue;
            };
            if unit != &window.unit {
                continue;
            }
            let holds: f64 = ledger
                .jobs
                .iter()
                .filter(|job| job.status != JobStatus::Released)
                .filter(|job| {
                    !matches!(
                        job.status,
                        JobStatus::Queued | JobStatus::Ready | JobStatus::Held
                    ) || !replaceable.contains(job.parent_task_id.as_str())
                })
                .flat_map(|job| &job.holds)
                .filter(|hold| {
                    hold.account_id == account.account_id && hold.window_id == window.window_id
                })
                .filter(|hold| {
                    hold.window_start.as_ref() == window.window_start.value.as_ref()
                        && hold.reset_at.as_ref() == window.reset_at.value.as_ref()
                })
                .filter(|hold| {
                    unreflected(hold.reflected_through.as_deref(), &snapshot.observed_at)
                })
                .map(|hold| hold.amount)
                .sum();
            let external: f64 = ledger
                .external_reserves
                .iter()
                .filter(|reserve| {
                    reserve.account_id == account.account_id
                        && reserve.window_id == window.window_id
                })
                .filter(|reserve| {
                    unreflected(reserve.reflected_through.as_deref(), &snapshot.observed_at)
                })
                .map(|reserve| reserve.amount)
                .sum();
            let available = snapshot.remaining - window.spent_since_snapshot - holds - external;
            ensure!(available.is_finite(), "Native capacity is not finite");
            capacity.insert(
                (
                    account.account_id.clone(),
                    window.window_id.clone(),
                    window.unit.clone(),
                ),
                available,
            );
        }
    }
    Ok(capacity)
}

fn unreflected(reflected_through: Option<&str>, snapshot_at: &str) -> bool {
    reflected_through != Some(snapshot_at)
}

fn unresolved_pilots(profile: &SetupProfile, ledger: &Ledger) -> BTreeSet<String> {
    ledger
        .jobs
        .iter()
        .filter(|job| job.status != JobStatus::Released)
        .flat_map(|job| &job.holds)
        .filter_map(|hold| {
            profile
                .native_coefficients
                .iter()
                .find(|coefficient| {
                    coefficient.id == hold.coefficient_id
                        && coefficient.sample_count == 0
                        && coefficient.manual_pilot
                })
                .map(|coefficient| format!("{}:{}", coefficient.account_id, coefficient.window_id))
        })
        .collect()
}

fn native_endowment(profile: &SetupProfile) -> BTreeMap<Resource, f64> {
    profile
        .accounts
        .iter()
        .flat_map(|account| {
            account.quota_windows.iter().filter_map(move |window| {
                window
                    .unit
                    .value
                    .as_ref()
                    .zip(window.capacity.value)
                    .map(|(unit, capacity)| {
                        (
                            (account.id.clone(), window.id.clone(), unit.clone()),
                            capacity,
                        )
                    })
            })
        })
        .collect()
}

fn existing_project_usage(
    ledger: &Ledger,
    replaceable: &BTreeSet<&str>,
) -> BTreeMap<String, BTreeMap<Resource, f64>> {
    let units: BTreeMap<_, _> = ledger
        .accounts
        .iter()
        .flat_map(|account| {
            account.windows.iter().map(move |window| {
                (
                    (account.account_id.as_str(), window.window_id.as_str()),
                    window.unit.as_str(),
                )
            })
        })
        .collect();
    let mut usage = BTreeMap::new();
    for job in ledger.jobs.iter().filter(|job| {
        !job.project_id.is_empty()
            && job.status != JobStatus::Released
            && (!matches!(
                job.status,
                JobStatus::Queued | JobStatus::Ready | JobStatus::Held
            ) || !replaceable.contains(job.parent_task_id.as_str()))
    }) {
        for hold in &job.holds {
            let current = ledger
                .accounts
                .iter()
                .find(|account| account.account_id == hold.account_id)
                .and_then(|account| {
                    account
                        .windows
                        .iter()
                        .find(|window| window.window_id == hold.window_id)
                });
            if current.is_some_and(|window| {
                hold.window_start.as_ref() == window.window_start.value.as_ref()
                    && hold.reset_at.as_ref() == window.reset_at.value.as_ref()
                    && window.snapshot.value.as_ref().is_some_and(|snapshot| {
                        unreflected(hold.reflected_through.as_deref(), &snapshot.observed_at)
                    })
            }) && let Some(unit) =
                units.get(&(hold.account_id.as_str(), hold.window_id.as_str()))
            {
                *usage
                    .entry(job.project_id.clone())
                    .or_insert_with(BTreeMap::new)
                    .entry((
                        hold.account_id.clone(),
                        hold.window_id.clone(),
                        (*unit).to_owned(),
                    ))
                    .or_default() += hold.amount;
            }
        }
        for settlement in &job.actual_settled {
            let current = ledger
                .accounts
                .iter()
                .find(|account| account.account_id == settlement.account_id)
                .and_then(|account| {
                    account
                        .windows
                        .iter()
                        .find(|window| window.window_id == settlement.window_id)
                });
            if current.is_some_and(|window| {
                settlement.window_start.as_ref() == window.window_start.value.as_ref()
                    && settlement.reset_at.as_ref() == window.reset_at.value.as_ref()
            }) {
                *usage
                    .entry(job.project_id.clone())
                    .or_insert_with(BTreeMap::new)
                    .entry((
                        settlement.account_id.clone(),
                        settlement.window_id.clone(),
                        settlement.unit.clone(),
                    ))
                    .or_default() += settlement.amount;
            }
        }
    }
    usage
}

fn active_windows(
    profile: &SetupProfile,
    ledger: &Ledger,
) -> Result<Vec<(OffsetDateTime, OffsetDateTime)>> {
    let windows = profile
        .workflow
        .active_windows
        .as_ref()
        .and_then(|fact| fact.value.as_ref())
        .context("Approved absolute active windows are unavailable")?;
    let now = OffsetDateTime::parse(&ledger.updated_at, &Rfc3339)
        .context("Ledger planning timestamp is invalid")?;
    let mut result = Vec::new();
    for window in windows {
        let start = OffsetDateTime::parse(&window.start, &Rfc3339)
            .context("Active window start is invalid")?
            .max(now);
        let end =
            OffsetDateTime::parse(&window.end, &Rfc3339).context("Active window end is invalid")?;
        if start < end {
            result.push((start, end));
        }
    }
    result.sort_by_key(|window| window.0);
    ensure!(
        result.windows(2).all(|pair| pair[0].1 <= pair[1].0),
        "Active windows overlap"
    );
    for job in ledger
        .jobs
        .iter()
        .filter(|job| job.status == JobStatus::Running)
    {
        let block_start = job
            .scheduled_start
            .as_ref()
            .map(|value| OffsetDateTime::parse(value, &Rfc3339))
            .transpose()
            .context("Running work start is invalid")?
            .unwrap_or(now);
        let block_end = if let Some(value) = &job.scheduled_end {
            OffsetDateTime::parse(value, &Rfc3339).context("Running work end is invalid")?
        } else if let Some(seconds) = job.remaining_seconds {
            ensure!(
                seconds.is_finite() && seconds >= 0.0,
                "Running work remaining time is invalid"
            );
            now + time::Duration::seconds_f64(seconds)
        } else {
            anyhow::bail!("Running work remaining time is unknown");
        };
        result = subtract_interval(result, block_start, block_end);
    }
    ensure!(
        !result.is_empty(),
        "No active time remains before the current native reset"
    );
    Ok(result)
}

fn subtract_interval(
    windows: Vec<(OffsetDateTime, OffsetDateTime)>,
    block_start: OffsetDateTime,
    block_end: OffsetDateTime,
) -> Vec<(OffsetDateTime, OffsetDateTime)> {
    windows
        .into_iter()
        .flat_map(|(start, end)| {
            if block_end <= start || block_start >= end {
                vec![(start, end)]
            } else {
                let mut pieces = Vec::new();
                if start < block_start {
                    pieces.push((start, block_start.min(end)));
                }
                if block_end < end {
                    pieces.push((block_end.max(start), end));
                }
                pieces
            }
        })
        .collect()
}

fn candidate_plans(
    task: &TaskRequest,
    routes: &[Route],
    profile: &SetupProfile,
    ledger: &Ledger,
    conductor: &str,
    limit: usize,
) -> Result<(Vec<CandidatePlan>, bool)> {
    let policy_task = policy_task(profile, task);
    let task = &policy_task;
    let related: Vec<_> = ledger
        .jobs
        .iter()
        .filter(|job| job.parent_task_id == task.task_id)
        .collect();
    let frozen: BTreeSet<_> = related
        .iter()
        .copied()
        .filter(|job| {
            matches!(
                job.status,
                JobStatus::Running | JobStatus::Completed | JobStatus::Blocked
            )
        })
        .map(|job| job.id.as_str())
        .collect();
    let repair_triggered = related.iter().any(|job| {
        job.status == JobStatus::Blocked
            && (job.seat == Some(Seat::Implementer)
                || matches!(job.seat, Some(Seat::Reviewer | Seat::Sanity))
                    && job.id.ends_with("-1"))
    });
    let initial_checks_accepted = [Seat::Reviewer, Seat::Sanity].into_iter().all(|seat| {
        related.iter().any(|job| {
            job.seat == Some(seat)
                && job.id.ends_with("-1")
                && job.status == JobStatus::Completed
                && job
                    .outcome
                    .as_ref()
                    .is_some_and(|outcome| outcome.artifact_accepted)
        })
    });
    let task_visits: Vec<_> = visits(task)
        .into_iter()
        .filter(|visit| {
            !frozen.contains(visit.id.as_str())
                && !(visit.conditional && initial_checks_accepted && !repair_triggered)
        })
        .collect();
    if task_visits.is_empty() {
        return Ok((Vec::new(), false));
    }
    let frozen_creator = ledger
        .jobs
        .iter()
        .find(|job| {
            job.parent_task_id == task.task_id
                && job.seat == Some(Seat::Implementer)
                && matches!(
                    job.status,
                    JobStatus::Running | JobStatus::Completed | JobStatus::Blocked
                )
        })
        .and_then(|job| {
            routes
                .iter()
                .find(|route| route.binding_id == job.binding_id)
        });
    let mut choices = Vec::new();
    for visit in &task_visits {
        let mut visit_choices = Vec::new();
        for route in routes {
            if visit.seat == Seat::Orchestrator && route.binding_id != conductor {
                continue;
            }
            if qualify(task, visit, route, profile, ledger).is_err() {
                continue;
            }
            let coefficients: Vec<_> = profile
                .native_coefficients
                .iter()
                .filter(|coefficient| {
                    coefficient.binding_id == route.binding_id
                        && coefficient.seat == visit.seat
                        && coefficient.workflow == task.effective_risk().template()
                })
                .cloned()
                .collect();
            if coefficient_bundle(task, route, &coefficients, profile) {
                visit_choices.push((route, coefficients));
            }
        }
        visit_choices.sort_by(|left, right| left.0.binding_id.cmp(&right.0.binding_id));
        choices.push(visit_choices);
    }
    let mut plans = Vec::new();
    assemble_plan(
        task,
        profile,
        conductor,
        &task_visits,
        &choices,
        frozen_creator,
        0,
        &mut Vec::new(),
        &mut plans,
        limit,
    );
    let limited = plans.len() >= limit;
    let now = OffsetDateTime::parse(&ledger.updated_at, &Rfc3339)
        .context("Ledger planning timestamp is invalid")?;
    for plan in &mut plans {
        plan.reset_at = plan
            .plan
            .assignments
            .iter()
            .flat_map(|assignment| &assignment.coefficients)
            .map(|coefficient| {
                ledger
                    .accounts
                    .iter()
                    .find(|account| account.account_id == coefficient.account_id)?
                    .windows
                    .iter()
                    .find(|window| window.window_id == coefficient.window_id)?
                    .reset_at
                    .value
                    .as_ref()
                    .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
            })
            .collect::<Option<Vec<_>>>()
            .and_then(|resets| resets.into_iter().min());
    }
    plans.retain(|plan| {
        let deadline_seconds = task
            .deadline
            .as_ref()
            .and_then(|deadline| OffsetDateTime::parse(deadline, &Rfc3339).ok())
            .map(|deadline| (deadline - now).as_seconds_f64())
            .unwrap_or(f64::INFINITY);
        plan.seconds <= task.timeout_seconds as f64
            && plan.seconds <= deadline_seconds
            && plan.reset_at.is_some()
    });
    Ok((plans, limited))
}

fn coefficient_bundle(
    task: &TaskRequest,
    route: &Route,
    coefficients: &[NativeCoefficient],
    profile: &SetupProfile,
) -> bool {
    let covered = profile
        .accounts
        .iter()
        .find(|account| account.id == route.account_id)
        .is_some_and(|account| {
            account.quota_windows.iter().all(|window| {
                !window.binding_ids.contains(&route.binding_id)
                    || window.unit.value.as_ref().is_some_and(|unit| {
                        coefficients.iter().any(|coefficient| {
                            coefficient.account_id == account.id
                                && coefficient.window_id == window.id
                                && coefficient.unit == *unit
                        })
                    })
            })
        });
    let calibrated = coefficients
        .iter()
        .all(|coefficient| coefficient.sample_count > 0);
    let pilot = coefficients
        .iter()
        .all(|coefficient| coefficient.sample_count > 0 || coefficient.manual_pilot)
        && task.effective_risk().consequence < 2
        && !matches!(
            task.effective_risk().template(),
            crate::portfolio::WorkClass::Complex | crate::portfolio::WorkClass::Extensive
        );
    !coefficients.is_empty() && covered && (calibrated || pilot)
}

#[expect(clippy::too_many_arguments)]
fn assemble_plan<'a>(
    task: &TaskRequest,
    profile: &SetupProfile,
    conductor: &str,
    task_visits: &[Visit],
    choices: &[Vec<(&'a Route, Vec<NativeCoefficient>)>],
    frozen_creator: Option<&'a Route>,
    index: usize,
    selected: &mut Vec<(&'a Route, Vec<NativeCoefficient>)>,
    plans: &mut Vec<CandidatePlan>,
    limit: usize,
) {
    if plans.len() >= limit || choices.get(index).is_some_and(Vec::is_empty) {
        return;
    }
    if index == task_visits.len() {
        let assignments: Vec<_> = task_visits
            .iter()
            .cloned()
            .zip(selected.iter())
            .map(|(visit, (route, coefficients))| Assignment {
                visit,
                route_id: route.id.clone(),
                binding_id: route.binding_id.clone(),
                account_id: route.account_id.clone(),
                coefficients: coefficients.clone(),
            })
            .collect();
        plans.push(candidate(task, conductor, assignments, profile));
        return;
    }
    for choice in &choices[index] {
        if checker_correlated(
            task,
            profile,
            task_visits,
            index,
            selected,
            frozen_creator,
            choice.0,
        ) {
            continue;
        }
        selected.push((choice.0, choice.1.clone()));
        assemble_plan(
            task,
            profile,
            conductor,
            task_visits,
            choices,
            frozen_creator,
            index + 1,
            selected,
            plans,
            limit,
        );
        selected.pop();
        if plans.len() >= limit {
            break;
        }
    }
}

fn checker_correlated(
    task: &TaskRequest,
    profile: &SetupProfile,
    task_visits: &[Visit],
    index: usize,
    selected: &[(&Route, Vec<NativeCoefficient>)],
    frozen_creator: Option<&Route>,
    proposed: &Route,
) -> bool {
    let visit = &task_visits[index];
    if !matches!(visit.seat, Seat::Reviewer | Seat::Sanity)
        || task.effective_risk().consequence < 2 && task.effective_risk().correlation < 2
    {
        return false;
    }
    let creator_seat = if visit.ordinal > 1 {
        Seat::Debugger
    } else {
        Seat::Implementer
    };
    let selected_creator = task_visits[..index]
        .iter()
        .zip(selected)
        .find_map(|(visit, choice)| (visit.seat == creator_seat).then_some(choice.0));
    let Some(creator) = selected_creator.or((creator_seat == Seat::Implementer)
        .then_some(frozen_creator)
        .flatten())
    else {
        return true;
    };
    same_identity(profile, creator, proposed)
}

fn same_identity(profile: &SetupProfile, left: &Route, right: &Route) -> bool {
    profile
        .bindings
        .iter()
        .find(|binding| binding.id == left.binding_id)
        .zip(
            profile
                .bindings
                .iter()
                .find(|binding| binding.id == right.binding_id),
        )
        .is_none_or(|(left, right)| {
            left.host_id == right.host_id || left.display_model == right.display_model
        })
}

fn candidate(
    task: &TaskRequest,
    conductor: &str,
    assignments: Vec<Assignment>,
    profile: &SetupProfile,
) -> CandidatePlan {
    let mut usage = BTreeMap::new();
    let mut account_seconds = BTreeMap::new();
    let mut host_seconds = BTreeMap::new();
    let mut seconds = 0.0;
    let mut pilot_keys = Vec::new();
    let mut visit_durations = Vec::new();
    let last = assignments.len().saturating_sub(1);
    for (index, assignment) in assignments.iter().enumerate() {
        let host = profile
            .bindings
            .iter()
            .find(|binding| binding.id == assignment.binding_id)
            .map(|binding| binding.host_id.clone())
            .unwrap_or_default();
        let mut duration = assignment
            .coefficients
            .iter()
            .map(|coefficient| coefficient.reserve_seconds)
            .fold(0.0, f64::max);
        if index == last {
            duration += task.human_seconds;
        }
        for coefficient in &assignment.coefficients {
            *usage
                .entry((
                    coefficient.account_id.clone(),
                    coefficient.window_id.clone(),
                    coefficient.unit.clone(),
                ))
                .or_default() += coefficient.reserve;
            if coefficient.sample_count == 0 && coefficient.manual_pilot {
                pilot_keys.push(format!(
                    "{}:{}",
                    coefficient.account_id, coefficient.window_id
                ));
            }
        }
        *account_seconds
            .entry(assignment.account_id.clone())
            .or_default() += duration;
        *host_seconds.entry(host).or_default() += duration;
        seconds += duration;
        visit_durations.push(duration);
    }
    let exclusive_seconds = task
        .required_resources
        .iter()
        .map(|resource| (resource.clone(), seconds))
        .collect();
    CandidatePlan {
        plan: NativePlan {
            policy_version: VERSION.to_owned(),
            task_id: task.task_id.clone(),
            project_id: task.project_id.clone(),
            workflow: task.effective_risk().template(),
            conductor_binding_id: conductor.to_owned(),
            assignments,
        },
        usage,
        account_seconds,
        host_seconds,
        exclusive_seconds,
        human_seconds: task.human_seconds,
        pilot_keys,
        visit_seconds: visit_durations,
        seconds,
        reset_at: None,
    }
}

impl Search<'_> {
    fn walk(&mut self, index: usize, selection: &mut [Option<usize>], state: &mut SearchState) {
        if self.nodes >= self.node_limit {
            self.limited = true;
            return;
        }
        self.nodes += 1;
        if index == self.tasks.len() {
            if !self.calendar_fits(selection) {
                return;
            }
            let score = self.score(selection, &state.project_counts);
            if score.compare(&self.incumbent_score) == Ordering::Greater {
                self.incumbent_score = score;
                self.incumbent.clone_from_slice(selection);
            }
            return;
        }
        for choice in 0..self.candidates[index].len() {
            let candidate = self.candidates[index][choice].clone();
            if self.fits(&candidate, state) {
                add(&candidate, state, 1.0);
                *state
                    .project_counts
                    .entry(self.tasks[index].project_id.clone())
                    .or_default() += 1;
                selection[index] = Some(choice);
                self.walk(index + 1, selection, state);
                selection[index] = None;
                *state
                    .project_counts
                    .get_mut(&self.tasks[index].project_id)
                    .unwrap() -= 1;
                add(&candidate, state, -1.0);
            }
            if self.limited {
                return;
            }
        }
        self.walk(index + 1, selection, state);
    }

    fn fits(&self, candidate: &CandidatePlan, state: &SearchState) -> bool {
        candidate.usage.iter().all(|(resource, amount)| {
            state.usage.get(resource).copied().unwrap_or(0.0) + amount
                <= self.capacity.get(resource).copied().unwrap_or(0.0) + 1e-9
        }) && candidate.account_seconds.iter().all(|(account, seconds)| {
            state.account_seconds.get(account).copied().unwrap_or(0.0) + seconds
                <= self.account_seconds_capacity + 1e-9
        }) && candidate.host_seconds.iter().all(|(host, seconds)| {
            state.host_seconds.get(host).copied().unwrap_or(0.0) + seconds
                <= self.host_seconds_capacity + 1e-9
        }) && candidate
            .exclusive_seconds
            .iter()
            .all(|(resource, seconds)| {
                state
                    .exclusive_seconds
                    .get(resource)
                    .copied()
                    .unwrap_or(0.0)
                    + seconds
                    <= self.horizon_seconds + 1e-9
            })
            && state.human_seconds + candidate.human_seconds <= self.human_seconds_capacity + 1e-9
            && candidate
                .pilot_keys
                .iter()
                .all(|key| !state.pilots.contains(key))
    }

    fn score(&self, selection: &[Option<usize>], _project_counts: &BTreeMap<String, u32>) -> Score {
        let mut score = Score::default();
        for (index, choice) in selection.iter().enumerate() {
            let Some(choice) = choice else { continue };
            let task = self.tasks[index];
            let risk = task.effective_risk();
            if risk.consequence >= 2 {
                score.high_consequence += u64::from(risk.consequence);
            }
            if let Some(deadline) = task
                .deadline
                .as_ref()
                .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
            {
                let slack = (deadline - self.planning_at).whole_seconds().max(0) as u64;
                score.deadline += (self.horizon_seconds.max(0.0) as u64)
                    .saturating_sub(slack)
                    .saturating_add(1);
            }
            score.useful += 1 + u64::from(
                self.project_priorities
                    .get(&task.project_id)
                    .copied()
                    .unwrap_or(0),
            );
            score.artifact_value += task.artifact_value;
            score.seconds += self.candidates[index][*choice].seconds;
        }
        score.fairness_cost = self.fairness_cost(selection);
        score.resource_cost = self.resource_cost(selection);
        score
    }

    fn fairness_cost(&self, selection: &[Option<usize>]) -> f64 {
        let mut project_usage = self.existing_project_usage.clone();
        let mut entitlement: BTreeMap<String, f64> = BTreeMap::new();
        for (index, choice) in selection.iter().enumerate() {
            let Some(choice) = choice else { continue };
            let task = self.tasks[index];
            entitlement.entry(task.project_id.clone()).or_insert(
                self.project_weights
                    .get(&task.project_id)
                    .copied()
                    .unwrap_or(1.0),
            );
            for (resource, amount) in &self.candidates[index][*choice].usage {
                *project_usage
                    .entry(task.project_id.clone())
                    .or_default()
                    .entry(resource.clone())
                    .or_default() += amount;
            }
        }
        project_usage
            .into_iter()
            .map(|(project, resources)| {
                let weight = entitlement.get(&project).copied().unwrap_or(1.0);
                resources
                    .into_iter()
                    .map(|(resource, amount)| {
                        self.endowment
                            .get(&resource)
                            .filter(|value| **value > 0.0)
                            .map_or_else(
                                || if amount > 0.0 { f64::INFINITY } else { 0.0 },
                                |value| amount / value / weight,
                            )
                    })
                    .fold(0.0, f64::max)
            })
            .fold(0.0, f64::max)
    }

    fn resource_cost(&self, selection: &[Option<usize>]) -> f64 {
        selection
            .iter()
            .enumerate()
            .filter_map(|(index, choice)| choice.map(|choice| &self.candidates[index][choice]))
            .flat_map(|candidate| &candidate.usage)
            .map(|(resource, amount)| {
                self.capacity
                    .get(resource)
                    .filter(|value| **value > 0.0)
                    .map_or(f64::INFINITY, |value| amount / value)
            })
            .sum()
    }

    fn calendar_fits(&self, selection: &[Option<usize>]) -> bool {
        let mut work: Vec<_> = selection
            .iter()
            .enumerate()
            .filter_map(|(index, choice)| {
                choice.map(|choice| {
                    let candidate = &self.candidates[index][choice];
                    (
                        self.tasks[index],
                        &candidate.visit_seconds,
                        candidate.reset_at,
                    )
                })
            })
            .collect();
        work.sort_by(|(left, _, _), (right, _, _)| {
            right
                .risk
                .consequence
                .cmp(&left.risk.consequence)
                .then_with(|| deadline_key(left).cmp(&deadline_key(right)))
                .then_with(|| {
                    self.project_priorities
                        .get(&right.project_id)
                        .cmp(&self.project_priorities.get(&left.project_id))
                })
        });
        let mut cursor = self.active_windows[0].0;
        let mut window_index = 0;
        for (task, visits, reset_at) in work {
            if let Some(ready_at) = &task.ready_at {
                let Ok(ready_at) = OffsetDateTime::parse(ready_at, &Rfc3339) else {
                    return false;
                };
                cursor = cursor.max(ready_at);
            }
            for seconds in visits {
                loop {
                    let Some((start, end)) = self.active_windows.get(window_index) else {
                        return false;
                    };
                    cursor = cursor.max(*start);
                    if (*end - cursor).as_seconds_f64() + 1e-9 >= *seconds {
                        break;
                    }
                    window_index += 1;
                }
                cursor += time::Duration::seconds_f64(*seconds);
            }
            if let Some(deadline) = &task.deadline {
                let Ok(deadline) = OffsetDateTime::parse(deadline, &Rfc3339) else {
                    return false;
                };
                if cursor > deadline {
                    return false;
                }
            }
            if reset_at.is_none_or(|reset| cursor > reset) {
                return false;
            }
        }
        true
    }
}

fn add(candidate: &CandidatePlan, state: &mut SearchState, direction: f64) {
    for (resource, amount) in &candidate.usage {
        *state.usage.entry(resource.clone()).or_default() += amount * direction;
    }
    for (account, seconds) in &candidate.account_seconds {
        *state.account_seconds.entry(account.clone()).or_default() += seconds * direction;
    }
    for (host, seconds) in &candidate.host_seconds {
        *state.host_seconds.entry(host.clone()).or_default() += seconds * direction;
    }
    for (resource, seconds) in &candidate.exclusive_seconds {
        *state.exclusive_seconds.entry(resource.clone()).or_default() += seconds * direction;
    }
    state.human_seconds += candidate.human_seconds * direction;
    for key in &candidate.pilot_keys {
        if direction > 0.0 {
            state.pilots.insert(key.clone());
        } else {
            state.pilots.remove(key);
        }
    }
}

fn dominant_shares(
    plans: &[NativePlan],
    endowment: &BTreeMap<Resource, f64>,
    existing: &BTreeMap<String, BTreeMap<Resource, f64>>,
    project_weights: &BTreeMap<String, f64>,
) -> Vec<DominantShare> {
    let mut usage = existing.clone();
    for plan in plans {
        for coefficient in plan
            .assignments
            .iter()
            .flat_map(|assignment| &assignment.coefficients)
        {
            *usage
                .entry(plan.project_id.clone())
                .or_default()
                .entry((
                    coefficient.account_id.clone(),
                    coefficient.window_id.clone(),
                    coefficient.unit.clone(),
                ))
                .or_default() += coefficient.reserve;
        }
    }
    usage
        .into_iter()
        .map(|(project_id, resources)| DominantShare {
            share: {
                let weight = project_weights.get(&project_id).copied().unwrap_or(1.0);
                resources
                    .into_iter()
                    .map(|(resource, amount)| {
                        endowment
                            .get(&resource)
                            .filter(|value| **value > 0.0)
                            .map_or_else(
                                || if amount > 0.0 { f64::INFINITY } else { 0.0 },
                                |value| amount / value / weight,
                            )
                    })
                    .fold(0.0, f64::max)
            },
            project_id,
        })
        .collect()
}

fn schedule(
    plans: &[NativePlan],
    tasks: &[&TaskRequest],
    profile: &SetupProfile,
    ledger: &Ledger,
    active_windows: &[(OffsetDateTime, OffsetDateTime)],
) -> Result<Vec<ScheduledVisit>> {
    let mut ordered: Vec<_> = plans.iter().collect();
    ordered.sort_by(|left, right| {
        let left_task = tasks
            .iter()
            .find(|task| task.task_id == left.task_id)
            .copied();
        let right_task = tasks
            .iter()
            .find(|task| task.task_id == right.task_id)
            .copied();
        right_task
            .map(|task| task.effective_risk().consequence)
            .cmp(&left_task.map(|task| task.effective_risk().consequence))
            .then_with(|| {
                left_task
                    .map_or(i128::MAX, deadline_key)
                    .cmp(&right_task.map_or(i128::MAX, deadline_key))
            })
            .then_with(|| {
                right_task
                    .map(|task| project_priority(profile, task))
                    .cmp(&left_task.map(|task| project_priority(profile, task)))
            })
    });
    let mut result = Vec::new();
    let mut cursor = active_windows[0].0;
    let mut window_index = 0;
    for plan in ordered {
        let task = tasks
            .iter()
            .find(|task| task.task_id == plan.task_id)
            .copied()
            .context("Scheduled task request is absent")?;
        if let Some(ready_at) = &task.ready_at {
            cursor = cursor.max(OffsetDateTime::parse(ready_at, &Rfc3339)?);
        }
        let last = plan.assignments.len().saturating_sub(1);
        for (index, assignment) in plan.assignments.iter().enumerate() {
            let mut seconds = assignment
                .coefficients
                .iter()
                .map(|coefficient| coefficient.reserve_seconds)
                .fold(0.0, f64::max);
            if index == last {
                seconds += task.human_seconds;
            }
            loop {
                let (start, end) = active_windows
                    .get(window_index)
                    .context("Selected plan does not fit the active calendar")?;
                cursor = cursor.max(*start);
                if (*end - cursor).as_seconds_f64() + 1e-9 >= seconds {
                    break;
                }
                window_index += 1;
            }
            let start = cursor;
            let end = start + time::Duration::seconds_f64(seconds);
            let binding = profile
                .bindings
                .iter()
                .find(|binding| binding.id == assignment.binding_id)
                .context("Scheduled binding is absent")?;
            let mut resources = vec![
                format!("account:{}", assignment.account_id),
                format!("host:{}", binding.host_id),
            ];
            if index == last && task.human_seconds > 0.0 {
                resources.push("human".to_owned());
            }
            resources.extend(task.required_resources.iter().cloned());
            resources.sort();
            resources.dedup();
            result.push(ScheduledVisit {
                task_id: plan.task_id.clone(),
                visit_id: assignment.visit.id.clone(),
                start: start.format(&Rfc3339)?,
                end: end.format(&Rfc3339)?,
                resources,
            });
            cursor = end;
        }
        let reset = plan
            .assignments
            .iter()
            .flat_map(|assignment| &assignment.coefficients)
            .filter_map(|coefficient| {
                ledger
                    .accounts
                    .iter()
                    .find(|account| account.account_id == coefficient.account_id)?
                    .windows
                    .iter()
                    .find(|window| window.window_id == coefficient.window_id)?
                    .reset_at
                    .value
                    .as_ref()
                    .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
            })
            .min()
            .context("Scheduled native reset is unavailable")?;
        ensure!(cursor <= reset, "Selected plan crosses its native reset");
    }
    Ok(result)
}

fn deadline_key(task: &TaskRequest) -> i128 {
    task.deadline
        .as_ref()
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
        .map_or(i128::MAX, OffsetDateTime::unix_timestamp_nanos)
}

fn window_pacing(
    plans: &[NativePlan],
    capacity: &BTreeMap<Resource, f64>,
) -> Vec<NativeWindowPacing> {
    let mut values: BTreeMap<Resource, (f64, f64, f64, f64)> = BTreeMap::new();
    for coefficient in plans
        .iter()
        .flat_map(|plan| &plan.assignments)
        .flat_map(|assignment| &assignment.coefficients)
    {
        let value = values
            .entry((
                coefficient.account_id.clone(),
                coefficient.window_id.clone(),
                coefficient.unit.clone(),
            ))
            .or_default();
        value.0 += coefficient.expected;
        value.1 += coefficient.reserve;
        value.2 += coefficient.uncertainty_lower;
        value.3 += coefficient.uncertainty_upper;
    }
    capacity
        .iter()
        .map(|((account_id, window_id, unit), available)| {
            let (expected, reserve, lower, upper) = values
                .get(&(account_id.clone(), window_id.clone(), unit.clone()))
                .copied()
                .unwrap_or_default();
            NativeWindowPacing {
                account_id: account_id.clone(),
                window_id: window_id.clone(),
                unit: unit.clone(),
                available: *available,
                planned_expected: expected,
                planned_reserve: reserve,
                uncertainty_lower: lower,
                uncertainty_upper: upper,
                remaining_slack: *available - reserve,
            }
        })
        .collect()
}
