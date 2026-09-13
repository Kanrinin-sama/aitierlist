use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

use crate::types::{Seat, Table};

pub const ROLE_ORDER: [Seat; 7] = [
    Seat::Orchestrator,
    Seat::Implementer,
    Seat::Debugger,
    Seat::Reviewer,
    Seat::Sanity,
    Seat::Comprehension,
    Seat::NetResearch,
];

pub fn isolated_protocol(table: &Table, paths: &crate::agent_setup::SetupPaths) -> String {
    let mut output = String::from(
        "# Isolated conductor\n\nYou own user communication, planning, delegation, and reconciliation. Use this generated protocol, the current user request, and fixed dispatcher routes. Do not import ambient user/project instructions, skills, plugins, MCP, hooks, or configuration. Read policy-index.json, then load only the selected role/class policy file on demand; do not load the full portable orchestrator document or schemas at routine boot. Native platform permissions and explicit user authority still apply.\n\n## Start or resume\n\n",
    );
    output.push_str(&format!("Read the compact approved profile at `{}` and live ledger at `{}` only as needed. If missing, incomplete, changed, or moved to another machine, consult the profile schema `{}` and ledger schema `{}` from disk; do not paste schemas into routine prompts. Verify isolated CLI/model/effort, accounts, subscription connectors, native quota/reset windows, work periods, and lifecycle controls. Batch missing facts for approval; reuse unchanged facts and scoped overrides. Installation proves neither auth nor approval. Inspect captured hosts, use fleet login <host-id> before finalization when authentication is needed, confirm authentication/profile facts, save approved profile/ledger, then finalize routes. Missing tools require manual or tool-enabled setup.\n\n", paths.profile.display(), paths.ledger.display(), paths.profile_schema.display(), paths.ledger_schema.display()));
    if let Some(portfolio) = &table.portfolio {
        output.push_str(&format!("Planning snapshot: {} concurrent project orchestrators, one conductor per project; {:.1} weekly wall-clock hours per account. Forecasts and weekly route reservations are aggregate across projects. The proxy calendar has one track per concurrent project, each bounded by the weekly work window; all tracks share the same account budgets. Native worker concurrency requires profile authorization. Confirm workflow.orchestrators separately from worker concurrency before spending.\n\n", portfolio.orchestrators, portfolio.available_hours_per_provider));
    }
    if let Some(rule) = recommended_route(table) {
        output.push_str(&format!("Recommend {} / {} as the fixed conductor allocation. A different host must preserve the exact model, effort, provider, plan, and account binding; otherwise replan before spending.\n", rule.provider_id, rule.plan_id));
    }
    output.push_str("## Risk-qualified workflows and useful work\n\nRecord consequence, uncertainty, coupling, reversibility, evidence need, tool risk, correlation, and deadline risk. Select the first matching workflow from Extensive to Focused. File count informs load only. Every worker seat has four conditional policies. The conductor remains fixed for the whole plan.\n");
    if let Some(portfolio) = &table.portfolio {
        for class in portfolio.dispatch.classes.iter().rev() {
            output.push_str(&format!("- {}: {}.\n", class.class.name(), class.condition));
        }
    }
    output.push_str("\nDelegate distinct needed briefs with owned paths, inputs, dependencies, acceptance evidence, and stop conditions. Use dispatcher route IDs/job schema, never native argv or auth. Parallelize disjoint writers and independent review when ready. Invoke only needed roles; inspect immutable results and reconcile compatible changes. Exit success alone is insufficient. Stop on acceptance or fixed attempt cap. Failure, scope growth, or exhausted capacity means replan/defer, never duplicate full tasks. Forecasts are not clock offsets or mandatory stages.\n\n## Shared pacing and task lifecycle\n\n");
    output.push_str("The approved collaboration root contains `workspace-index.v1.json`. Use its projects, briefs, decisions, research, deliverables, and ignored scratch paths for user work. Keep profiles, authentication, ledgers, holds, and generated runtime state in the app-owned directory. Git history belongs to the collaboration root; never commit or push automatically.\n\n");
    output.push_str(&format!("Submit authorized ready tasks only through the generated `fleet` commands. Use `fleet admit task <task-json>` to add work and jointly replan, `fleet run <visit-id>` to launch the assigned fixed route after dependency and identity checks, `fleet status <task-or-visit-id>`, `fleet settle <visit-id> <settlement-json>`, and `fleet cancel <task-id>`. Read `native-plan.v1.json` for the current whole-horizon policy. The dispatcher alone acquires `{}`, computes native availability, writes reservations and settlements, and mutates the ledger. Never edit the ledger or perform quota arithmetic in the conductor. All aliases, roles, and harnesses share real account/window balances, including conductor calls. Availability includes authoritative remaining capacity minus only unreflected residual holds and unreflected external use or reserve. Cancellation releases only never-started holds; started unknown usage remains until dispatcher reconciliation.\n\n", paths.lock.display()));
    output.push_str("At first admission and each dispatch, completion, reset, or material scope change, submit the ready queue and current deadlines to the dispatcher. It evaluates every binding native window using calibrated uncertainty, reset timing, the work calendar, dependencies, and deadline slack. A deficit uses the listed lower-effort route, then a listed qualified substitution, authorized scope reduction, or deferral. Spare capacity serves only independent needed work without critical-path delay. Unknown meters permit only the bounded pilot encoded by policy pending manual reconciliation. Finish when useful demand ends; report unused quota.\n\nTrack ready/running/blocked/completed/cancelled work through fleet status. Cancel owned workers through the dispatcher. Resume from recorded status and artifacts before any duplicate dispatch. Keep changes in the generated workspace until the app's explicit reconciliation path authorizes transfer. Finish with concise results, evidence, and remaining limits.\n");
    output
}

pub fn write_policy(table: &Table, path: &Path) -> Result<()> {
    let markdown = render(table)?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(markdown.as_bytes())
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn research_route_name(row: &crate::types::Row, native_harness: &str) -> String {
    if native_harness
        .to_ascii_lowercase()
        .starts_with("antigravity")
    {
        return match row.effort.as_deref() {
            Some(effort) if !effort.is_empty() => format!("AGY - {} ({effort})", row.model),
            _ => format!("AGY - {}", row.model),
        };
    }
    row.display_name()
}

pub fn recommended_route(table: &Table) -> Option<&crate::portfolio::ConductorAllocation> {
    table.portfolio.as_ref()?.conductor.as_ref()
}

fn render_native_plan(plan: &crate::team_policy::NativePlan) -> Result<String> {
    anyhow::ensure!(
        plan.policy_version == crate::team_policy::VERSION,
        "Native plan policy version does not match this executable"
    );
    anyhow::ensure!(
        plan.assignments
            .iter()
            .all(|assignment| !assignment.coefficients.is_empty()),
        "Native plan contains an assignment without a funded native coefficient"
    );
    let mut output = format!(
        "## Native admitted plan\n\nPolicy `{}`. Project `{}`. Task `{}`. Workflow `{}`. Fixed conductor binding `{}`. This table is the executable native assignment for one authorized ready task.\n\n| Visit ID | Seat | Conditional | Route ID | Binding ID | Account ID | Native reservation | Calibration |\n|---|---|---|---|---|---|---|---|\n",
        cell(&plan.policy_version),
        cell(&plan.project_id),
        cell(&plan.task_id),
        plan.workflow.name(),
        cell(&plan.conductor_binding_id)
    );
    for assignment in &plan.assignments {
        let reservation = if assignment.coefficients.is_empty() {
            "bounded manual pilot; exact meter hold unresolved".to_owned()
        } else {
            assignment
                .coefficients
                .iter()
                .map(|coefficient| {
                    format!(
                        "{} {} in {}",
                        coefficient.reserve, coefficient.unit, coefficient.window_id
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        let calibration = if assignment.coefficients.is_empty() {
            "uncalibrated".to_owned()
        } else {
            assignment
                .coefficients
                .iter()
                .map(|coefficient| {
                    format!(
                        "n={}, {}..{}, {}",
                        coefficient.sample_count,
                        coefficient.uncertainty_lower,
                        coefficient.uncertainty_upper,
                        coefficient.uncertainty
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        };
        writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            cell(&assignment.visit.id),
            assignment.visit.seat.name(),
            assignment.visit.conditional,
            cell(&assignment.route_id),
            cell(&assignment.binding_id),
            cell(&assignment.account_id),
            cell(&reservation),
            cell(&calibration)
        )?;
    }
    writeln!(
        output,
        "\nPlanned integer visit volume: {}. Qualification was evaluated against current profile, ledger, billing, tools, permissions, source scope, dependencies, and native coefficients. Any affected identity or evidence change invalidates this plan and requires joint replanning.",
        plan.assignments.len()
    )?;
    Ok(output)
}

pub fn render_native_horizon(horizon: &crate::native_scheduler::NativeHorizon) -> Result<String> {
    anyhow::ensure!(
        horizon.policy_version == crate::team_policy::VERSION,
        "Native horizon policy version does not match this executable"
    );
    anyhow::ensure!(
        horizon
            .plans
            .iter()
            .flat_map(|plan| &plan.assignments)
            .all(|assignment| !assignment.coefficients.is_empty()),
        "Native horizon contains an assignment without a funded native coefficient"
    );
    let mut output = format!(
        "# Native horizon\n\nPolicy `{}`. Horizon `{}`. Profile `{}`. Ledger revision {}. Search `{:?}` after {} nodes; feasible gap {}. {} tasks admitted and {} deferred.\n\n",
        cell(&horizon.policy_version),
        cell(&horizon.horizon_id),
        cell(&horizon.profile_identity),
        horizon.ledger_revision,
        horizon.search_status,
        horizon.explored_nodes,
        horizon
            .feasible_gap
            .map(|gap| format!("{:.2}%", gap * 100.0))
            .unwrap_or_else(|| "unavailable".to_owned()),
        horizon.plans.len(),
        horizon.deferred.len()
    );
    output.push_str(
        "## Whole-plan conductor\n\n| Binding ID | Planned integer visits |\n|---|---:|\n",
    );
    let mut conductors = std::collections::BTreeMap::<&str, usize>::new();
    for plan in &horizon.plans {
        for assignment in plan
            .assignments
            .iter()
            .filter(|assignment| assignment.visit.seat == Seat::Orchestrator)
        {
            *conductors.entry(&assignment.binding_id).or_default() += 1;
        }
    }
    for (binding, visits) in conductors {
        writeln!(output, "| {} | {} |", cell(binding), visits)?;
    }
    output.push_str("\n## Worker allocation\n\n| Role | Workflow | Route ID | Binding ID | Account ID | Planned integer visits |\n|---|---|---|---|---|---:|\n");
    let mut groups =
        std::collections::BTreeMap::<(String, String, String, String, String), usize>::new();
    for plan in &horizon.plans {
        for assignment in plan
            .assignments
            .iter()
            .filter(|assignment| assignment.visit.seat != Seat::Orchestrator)
        {
            *groups
                .entry((
                    assignment.visit.seat.name().to_owned(),
                    plan.workflow.name().to_owned(),
                    assignment.route_id.clone(),
                    assignment.binding_id.clone(),
                    assignment.account_id.clone(),
                ))
                .or_default() += 1;
        }
    }
    for ((seat, workflow, route, binding, account), visits) in groups {
        writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} |",
            cell(&seat),
            cell(&workflow),
            cell(&route),
            cell(&binding),
            cell(&account),
            visits
        )?;
    }
    output.push_str("\n## Task assignments\n\n");
    for plan in &horizon.plans {
        output.push_str(&render_native_plan(plan)?);
        output.push('\n');
    }
    if !horizon.deferred.is_empty() {
        output.push_str("## Deferred tasks\n\n| Task ID | Reason |\n|---|---|\n");
        for task in &horizon.deferred {
            writeln!(
                output,
                "| {} | {} |",
                cell(&task.task_id),
                cell(&task.reason)
            )?;
        }
    }
    if !horizon.dominant_shares.is_empty() {
        output.push_str("\n## Project dominant shares\n\n| Project ID | Share |\n|---|---:|\n");
        for share in &horizon.dominant_shares {
            writeln!(
                output,
                "| {} | {:.4} |",
                cell(&share.project_id),
                share.share
            )?;
        }
    }
    if !horizon.scheduled_visits.is_empty() {
        output.push_str("\n## Native schedule\n\n| Task ID | Visit ID | Start | End | Resources |\n|---|---|---|---|---|\n");
        for visit in &horizon.scheduled_visits {
            writeln!(
                output,
                "| {} | {} | {} | {} | {} |",
                cell(&visit.task_id),
                cell(&visit.visit_id),
                cell(&visit.start),
                cell(&visit.end),
                cell(&visit.resources.join(", "))
            )?;
        }
    }
    if !horizon.window_pacing.is_empty() {
        output.push_str("\n## Native window forecast\n\n| Account | Window | Unit | Available | Expected | Reserve | Uncertainty | Remaining slack |\n|---|---|---|---:|---:|---:|---:|---:|\n");
        for window in &horizon.window_pacing {
            writeln!(
                output,
                "| {} | {} | {} | {:.4} | {:.4} | {:.4} | {:.4}..{:.4} | {:.4} |",
                cell(&window.account_id),
                cell(&window.window_id),
                cell(&window.unit),
                window.available,
                window.planned_expected,
                window.planned_reserve,
                window.uncertainty_lower,
                window.uncertainty_upper,
                window.remaining_slack
            )?;
        }
    }
    Ok(output)
}

pub fn render(table: &Table) -> Result<String> {
    let discovery = crate::host_install::discover(table)?;
    render_with_discovery(table, &discovery)
}

pub fn render_with_discovery(
    table: &Table,
    discovery: &crate::host_install::Discovery,
) -> Result<String> {
    let portfolio = table
        .portfolio
        .as_ref()
        .context("No role policy is available")?;
    anyhow::ensure!(
        portfolio.dispatch.executable,
        "No usable role policy: {}",
        portfolio.message
    );
    anyhow::ensure!(
        portfolio.conductor.is_some(),
        "Regenerate this plan to select one fixed Orchestrator"
    );
    let mut output = String::from("# Orchestrator\n\n");
    output.push_str(CONDUCTOR);
    writeln!(
        output,
        "\n## Planning snapshot\n\nPolicy `{}` from aitierlist {}. Benchmark snapshot {} ({}). Model-estimate working window {:.1} hours per account; {} concurrent project orchestrators. One conductor per project shares all account balances and worker gates. The proxy forecast contains {} integer jobs and the proxy solver fits {} under API-equivalent assumptions. The proxy allocation is not runtime launch permission or a calendar.\n",
        portfolio.dispatch.policy_version,
        env!("CARGO_PKG_VERSION"),
        table.source_fetched_at,
        table.cache_state.name(),
        portfolio.available_hours_per_provider,
        portfolio.orchestrators,
        portfolio.dispatch.forecast_changes,
        portfolio.dispatch.admitted_changes
    )?;
    if let Some(gap) = portfolio.dispatch.solver.relative_gap {
        writeln!(
            output,
            "Static admitted-work MCDA utility gap <= {:.2}% under the declared scheduler and numerical tolerances. This says nothing about adaptive runtime optimality or exact quota exhaustion.\n",
            gap * 100.0
        )?;
    }
    if let Some(route) = recommended_route(table) {
        writeln!(
            output,
            "Recommended human-facing orchestrator: {} / {}. Its exact model and effort are fixed for every project and work class.\n",
            route.provider_id, route.plan_id
        )?;
    }
    output.push_str("## Risk-qualified workflow templates\n\nRecord the eight risk-vector dimensions and evaluate from Extensive to Focused; first match wins. File count informs load and never selects a template. Research eligibility depends provisionally on verified source tools, billing, permissions, and task-relevant evidence; it has no coding-benchmark prerequisite and no fabricated success probability.\n\n");
    for class in portfolio.dispatch.classes.iter().rev() {
        writeln!(
            output,
            "- {}: {}. Research reference: {}.",
            class.class.name(),
            class.condition,
            if class.research_included {
                "included in every static proxy job"
            } else {
                "excluded from the coding-only static proxy"
            }
        )?;
    }
    output.push_str("\n## Exact model registry\n\nIDs identify native harness/model/effort combinations, not aliases with independent quota.\n\n| ID | Benchmark harness | Native harness | Model | Effort | Model provider | Profile binding ID |\n|---|---|---|---|---|---|---|\n");
    let mut registry = std::collections::BTreeSet::new();
    for rule in portfolio
        .roles
        .iter()
        .filter(|role| role.seat != Seat::Orchestrator)
        .flat_map(|role| &role.rules)
    {
        if let Some(index) = rule.row_index {
            registry.insert(index);
        }
        for alternative in rule
            .lower_effort
            .iter()
            .chain(&rule.within_provider_alternatives)
            .chain(&rule.surplus_alternatives)
        {
            registry.insert(alternative.row_index);
        }
    }
    let mut registered_bindings = std::collections::BTreeSet::new();
    for index in registry {
        let row = table.rows.get(index).context("Policy model unavailable")?;
        let binding_id = crate::agent_setup::binding_id(row);
        if !registered_bindings.insert(binding_id.clone()) {
            continue;
        }
        writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} | {} |",
            cell(&crate::team_policy::route_id(&binding_id)),
            cell(&row.harness),
            cell(&row.harness),
            cell(&row.model),
            cell(row.effort.as_deref().unwrap_or("unspecified")),
            cell(&row.vendor),
            cell(&binding_id)
        )?;
    }
    for candidate in portfolio
        .roles
        .iter()
        .filter(|role| role.seat == Seat::NetResearch)
        .flat_map(|role| &role.rules)
        .flat_map(|rule| &rule.research_candidates)
    {
        if !registered_bindings.insert(candidate.binding_id.clone()) {
            continue;
        }
        let row = table
            .rows
            .get(candidate.row_index)
            .context("Research benchmark row unavailable")?;
        writeln!(
            output,
            "| {} | {} | {} | {} | {} | {} | {} |",
            cell(&crate::team_policy::route_id(&candidate.binding_id)),
            cell(&row.harness),
            cell(&candidate.native_harness),
            cell(&row.model),
            cell(row.effort.as_deref().unwrap_or("unspecified")),
            cell(&candidate.provider_id),
            cell(&candidate.binding_id)
        )?;
    }
    if let Some(conductor) = &portfolio.conductor {
        let row = table
            .rows
            .get(conductor.row_index)
            .context("Conductor benchmark row unavailable")?;
        writeln!(
            output,
            "| conductor | {} | {} | {} | {} | {} | {} |",
            cell(&row.harness),
            cell(&conductor.native_harness),
            cell(&row.model),
            cell(row.effort.as_deref().unwrap_or("unspecified")),
            cell(&conductor.provider_id),
            cell(&conductor.binding_id)
        )?;
    }
    output.push_str("\n## Persistent orchestrator\n\nThe same exact model and effort provides a persistent human-facing coordination service across concurrent projects and work classes. Each admitted worker job charges one measured dispatch at the stress-adjusted per-job USD rate through the applicable windows. Continuous/idle use between dispatches remains unmeasured and uncharged beyond any user allowance. Optional USD headroom is an additional weekly allowance for the whole fleet, reserved once on top of dispatch spend; hour headroom is reserved separately. Actual native calls are governed by ledger holds. Dispatch resource values are benchmark-derived forecasts, not native quota claims. Benchmark provenance and native launch binding remain explicit.\n\n| Service | Model ID | Benchmark source | Account / plan | Cap | Capability | Utility | Evidence profile | II / LCR / HLE | Usage forecast | Additional weekly fleet idle allowance USD / hours reserve | Per-dispatch reserved USD / h |\n|---|---|---|---|---:|---:|---:|---|---|---|---|---|\n");
    if let Some(conductor) = &portfolio.conductor {
        let benchmark = table
            .rows
            .get(conductor.row_index)
            .context("Conductor benchmark row unavailable")?;
        let forecast = conductor.usage_forecast.as_ref().map_or_else(
            || "On demand / unknown".to_owned(),
            |forecast| {
                format!(
                    "{} calls / {:.6} USD / {:.6} h",
                    forecast.visits, forecast.usage, forecast.hours
                )
            },
        );
        let headroom = format!(
            "{} / {}",
            conductor
                .headroom
                .usd
                .map_or_else(|| "unspecified".to_owned(), |value| format!("{value:.6}")),
            conductor
                .headroom
                .hours
                .map_or_else(|| "unspecified".to_owned(), |value| format!("{value:.6}"))
        );
        let component = |value: Option<f64>| {
            value.map_or_else(|| "Unknown".to_owned(), |value| format!("{value:.3}"))
        };
        let evidence_components = format!(
            "{} / {} / {}",
            component(benchmark.smart),
            component(benchmark.lcr),
            component(benchmark.hle)
        );
        writeln!(
            output,
            "| {} | conductor | {} / {} / {} | {} / {} | {} | {:.3} | {:.4} | {} | {} | {} | {} | {:.6} / {:.6} |",
            if conductor.persistent {
                "Persistent / On demand"
            } else {
                "On demand"
            },
            cell(&benchmark.harness),
            cell(&benchmark.model),
            cell(benchmark.effort.as_deref().unwrap_or("unspecified")),
            cell(&conductor.provider_id),
            cell(&conductor.plan_id),
            conductor.attempt_limit,
            conductor.competence,
            conductor.utility,
            cell(&conductor.evidence_profile),
            cell(&evidence_components),
            cell(&forecast),
            cell(&headroom),
            conductor.per_call_reserved_usage,
            conductor.per_call_reserved_hours
        )?;
    } else {
        output.push_str("| On demand | Unfunded | - | - | - | - | - | - | - | - | - | - |\n");
    }
    let service = &portfolio.dispatch.service;
    writeln!(
        output,
        "\n## Quality-adjusted worker service\n\nThe chosen worker plan provides {:.6} quality-adjusted service over workload W {:.6}, with geometric team capability exp(L/W) {:.6}. It admits {} jobs; the largest worker plan found admits {} jobs and provides {} service. The objective is policy utility, not a probability of task success. The worker plan is conditional on the persistent human-facing orchestrator service.\n\nSearch status: {}. Objective bound: {}. Relative gap: {}. {}\n",
        service.quality_adjusted_service,
        service.workload,
        service.team_capability,
        portfolio.dispatch.admitted_changes,
        service.maximum_volume_admitted,
        service
            .maximum_volume_service
            .map_or_else(|| "Unknown".to_owned(), |value| format!("{value:.6}")),
        if service.proven_optimal {
            "proved"
        } else {
            "unresolved"
        },
        service
            .bound
            .map_or_else(|| "Unknown".to_owned(), |value| format!("{value:.6}")),
        service.relative_gap.map_or_else(
            || "Unknown".to_owned(),
            |value| format!("{:.2}%", value * 100.0)
        ),
        service.message
    )?;
    output.push_str("\n## Conditional worker policy\n\nEach seat has four workflow rows. A row names one default exact binding, its planned integer proxy volume, and its account claim. Expand its fallback only when the stated eligibility or native-window condition occurs; never round-robin among routes. Caps bound attempts for one distinct assignment and stop on acceptance.\n\n| Policy ID | Role | Template | Planned jobs | Route ID | Exact account claim | Cap | Evidence status | Proxy USD per visit | Proxy h per visit |\n|---|---|---|---:|---|---|---:|---|---:|---:|\n");
    for seat in ROLE_ORDER
        .into_iter()
        .filter(|seat| *seat != Seat::Orchestrator)
    {
        let role = portfolio
            .roles
            .iter()
            .find(|role| role.seat == seat)
            .context("Policy role unavailable")?;
        for rule in &role.rules {
            if let Some(index) = rule.row_index {
                writeln!(
                    output,
                    "| {} | {} | {} | {} | {} | {} | {} | {} | {:.6} / {:.6} | {:.6} / {:.6} |",
                    cell(&rule.policy_id),
                    seat.name(),
                    rule.class.name(),
                    rule.planned_jobs,
                    cell(&crate::team_policy::route_id(
                        &crate::agent_setup::binding_id(&table.rows[index])
                    )),
                    cell(&rule.account_claim),
                    rule.attempt_limit,
                    cell(&rule.calibration),
                    rule.per_call_expected_usage,
                    rule.per_call_reserved_usage,
                    rule.per_call_expected_hours,
                    rule.per_call_reserved_hours
                )?;
            } else {
                if seat == Seat::NetResearch && rule.research_candidates.is_empty() {
                    writeln!(
                        output,
                        "| {} | {} | {} | On demand | No subscribed candidate | Conditional native binding | - | {} | Unmeasured | Unmeasured |",
                        cell(&rule.policy_id),
                        seat.name(),
                        rule.class.name(),
                        cell(&rule.calibration)
                    )?;
                } else if rule.research_candidates.is_empty() {
                    writeln!(
                        output,
                        "| {} | {} | {} | 0 | Unfunded | Unfunded | - | {} | - | - |",
                        cell(&rule.policy_id),
                        seat.name(),
                        rule.class.name(),
                        cell(&rule.calibration)
                    )?;
                } else {
                    let primary = rule
                        .research_candidates
                        .iter()
                        .find(|candidate| candidate.primary)
                        .unwrap_or(&rule.research_candidates[0]);
                    writeln!(
                        output,
                        "| {} | {} | {} | On demand | {} | {} / {} | On demand | AA score {:.4}; {} | {} | {} |",
                        cell(&rule.policy_id),
                        seat.name(),
                        rule.class.name(),
                        cell(&crate::team_policy::route_id(&primary.binding_id)),
                        primary.provider_id,
                        primary.plan_id,
                        primary.score,
                        cell(&primary.eligibility),
                        primary
                            .expected_usd
                            .map(|value| format!("{value:.6}"))
                            .unwrap_or_else(|| "Unknown".to_string()),
                        primary
                            .decode_hours
                            .map(|value| format!("{value:.6}"))
                            .unwrap_or_else(|| "Unknown".to_string())
                    )?;
                }
            }
        }
    }
    output.push_str("\n## Net Research ranking policy\n\nEach template ranks routes by `score = (accuracy^w_accuracy × no incorrect answer (all questions)^w_non_wrong × LCR^w_LCR × HLE^w_HLE)^(1 / sum(weights)), omitting zero-weight components`. The funded default is the route selected by the joint schedule; without funded research, the highest score leads the conditional recommendations. Weights are task-policy preferences, not probabilities. GPQA and GDP.pdf are diagnostics. Expected USD and decode hours are AA reference-workload proxies, not native research latency, budget, quota, or holds. Every route remains conditional on task-specific tools, permissions, source scope, billing, and a native hold.\n\n| Template | Rank | Route | Native harness | Account / plan | Score | Weighted components | Diagnostics | AA proxy USD / decode h | Eligibility | Source |\n|---|---:|---|---|---|---:|---|---|---|---|---|\n");
    if let Some(role) = portfolio
        .roles
        .iter()
        .find(|role| role.seat == Seat::NetResearch)
    {
        for rule in &role.rules {
            for (rank, candidate) in rule.research_candidates.iter().enumerate() {
                let row = table
                    .rows
                    .get(candidate.row_index)
                    .context("Research candidate unavailable")?;
                writeln!(
                    output,
                    "| {} | {} | {} ({}) | {} | {} / {} | {:.4} | accuracy {:.4} x {:.2}; no incorrect answer (all questions) {:.4} x {:.2}; LCR {:.4} x {:.2}; HLE {} x {:.2} | GPQA {}; GDP.pdf {} | {} / {} | {} | {}; [methodology](https://artificialanalysis.ai/methodology/intelligence-benchmarking) |",
                    rule.class.name(),
                    if candidate.primary {
                        "Default".to_string()
                    } else {
                        format!("Alternative {}", rank)
                    },
                    cell(&research_route_name(row, &candidate.native_harness)),
                    cell(&crate::team_policy::route_id(&candidate.binding_id)),
                    cell(&candidate.native_harness),
                    candidate.provider_id,
                    candidate.plan_id,
                    candidate.score,
                    candidate.accuracy,
                    candidate.accuracy_weight,
                    candidate.non_wrong,
                    candidate.non_wrong_weight,
                    candidate.lcr,
                    candidate.lcr_weight,
                    candidate
                        .hle
                        .map(|value| format!("{value:.4}"))
                        .unwrap_or_else(|| "n/a".to_string()),
                    candidate.hle_weight,
                    candidate
                        .gpqa_diagnostic
                        .map(|value| format!("{value:.4}"))
                        .unwrap_or_else(|| "n/a".to_string()),
                    candidate
                        .gdp_pdf_diagnostic
                        .map(|value| format!("{value:.4}"))
                        .unwrap_or_else(|| "n/a".to_string()),
                    candidate
                        .expected_usd
                        .map(|value| format!("{value:.4}"))
                        .unwrap_or_else(|| "Unknown".to_string()),
                    candidate
                        .decode_hours
                        .map(|value| format!("{value:.4}"))
                        .unwrap_or_else(|| "Unknown".to_string()),
                    cell(&candidate.eligibility),
                    cell(&candidate.source)
                )?;
            }
        }
    }
    output.push_str("\n## Expandable fallback policy\n\nFallbacks do not extend the proxy reservation or prove native feasibility. The dispatcher considers them in the listed condition order for the same authorized ready job, verifies exact eligibility and account funding, and recomputes native holds. Benchmark scores are provisional evidence dimensions rather than universal thresholds or probabilities.\n\n| Role / template | Trigger | Route ID | Account / plan | Cap | Evidence score | MCDA utility | Regret | Opportunity cost | Proxy expected USD / h | Proxy reserve USD / h |\n|---|---|---|---|---:|---:|---:|---:|---:|---|---|\n");
    for seat in ROLE_ORDER
        .into_iter()
        .filter(|seat| *seat != Seat::Orchestrator)
    {
        if let Some(role) = portfolio.roles.iter().find(|role| role.seat == seat) {
            for rule in &role.rules {
                for (purpose, route) in rule
                    .lower_effort
                    .iter()
                    .map(|route| ("Lower effort", route))
                    .chain(
                        rule.within_provider_alternatives
                            .iter()
                            .map(|route| ("Same account", route)),
                    )
                    .chain(
                        rule.surplus_alternatives
                            .iter()
                            .map(|route| ("Surplus", route)),
                    )
                {
                    writeln!(
                        output,
                        "| {} / {} | {} | {} | {} / {} | {} | {:.3} | {:.4} | {:.4} | {:.4} | {:.6} / {:.6} | {:.6} / {:.6} |",
                        seat.name(),
                        rule.class.name(),
                        purpose,
                        cell(&crate::team_policy::route_id(&route.binding_id)),
                        route.provider_id,
                        route.plan_id,
                        route.attempt_limit,
                        route.competence,
                        route.quality,
                        route.regret,
                        route.opportunity_cost,
                        route.per_call_expected_usage,
                        route.per_call_expected_hours,
                        route.per_call_reserved_usage,
                        route.per_call_reserved_hours
                    )?;
                }
            }
        }
    }
    writeln!(
        output,
        "\n## Math and allocation\n\n{} {} Orchestrator modeled spend share: {:.1}% expected / {:.1}% reserved. Classes scale reference volume, not calibrated difficulty; cadence and repair incidence are assumptions. Full repair/cap reservations are stress buffers that can be released as work resolves, not a target for idle quota.\n\n| Role | Account | Expected / reserved USD | Expected / reserved h |\n|---|---|---:|---:|",
        portfolio.math_audit.objective,
        portfolio.math_audit.coordination_proxy,
        portfolio.math_audit.orchestrator_expected_spend_share * 100.0,
        portfolio.math_audit.orchestrator_reserved_spend_share * 100.0
    )?;
    for usage in &portfolio.math_audit.role_accounts {
        writeln!(
            output,
            "| {} | {} | {:.4} / {:.4} | {:.4} / {:.4} |",
            usage.seat.name(),
            usage.provider_id,
            usage.expected_usage,
            usage.reserved_usage,
            usage.expected_hours,
            usage.reserved_hours
        )?;
    }
    output.push_str("\n## Shared account planning inputs\n\nAPI-equivalent USD below is a model input, never a native quota meter. The profile binds one account per provider; its forecast allowance is shared by one H-hour calendar track per parallel orchestrator, even when more subscriptions are owned. Onboarding must resolve real account aliases, entitlement, meter units, resets, and remaining working window before using them operationally.\n\n");
    for pool in &portfolio.pools {
        writeln!(
            output,
            "- {} / {}: one bound account contributes modeled {} USD/week and {:.1} aggregate track hours/week; {} owned subscriptions count toward monthly purchase cost only and add no forecast capacity; [plan source]({}). {}",
            pool.provider_id,
            pool.plan_id,
            pool.per_account_weekly_capacity
                .map_or_else(|| "unknown".to_owned(), |capacity| format!("{capacity:.4}")),
            pool.available_hours,
            pool.subscription_count,
            pool.source_url,
            pool.allowance_basis
        )?;
    }
    output.push_str("\n<!-- aitierlist:onboarding:start -->\n");
    output.push_str(if discovery.setup.ready {
        REUSE_PROFILE
    } else {
        ONBOARDING
    });
    if !discovery.setup.ready {
        let hints = discovery.hosts.iter().filter(|host| host.detection != "absent" || host.installed || host.executable.is_some() || !host.environment.is_empty() || !host.arguments.is_empty())
            .map(|host| serde_json::json!({
                "id": host.id,
                "kind": host.kind,
                "detection": host.detection,
                "detected_path": host.detected_path,
                "executable": host.executable,
                "argv": host.arguments,
                "config_home_env": host.environment,
                "status": if host.installed && host.executable.is_some() { "detected" } else { "setup_needed" }
            })).collect::<Vec<_>>();
        writeln!(
            output,
            "\nDiscovery snapshot hints to verify, not authentication, entitlement, or native-model facts. Empty config-home environment means defaults still need confirmation.\n```json\n{}\n```",
            serde_json::to_string(&hints)?
        )?;
    }
    if let Some(profile) = &discovery.setup.profile {
        writeln!(
            output,
            "\n{} Preserve confirmed facts and intentional overrides; resolve only missing or changed facts.\n```json\n{}\n```",
            if discovery.setup.ready {
                "Reusable approved machine profile."
            } else {
                "Existing partial machine profile; onboarding is incomplete."
            },
            serde_json::to_string(profile)?
        )?;
    } else {
        writeln!(
            output,
            "\nNo reusable approved machine profile. Explicit-unknown starter:\n```json\n{}\n```",
            crate::agent_setup::onboarding_example()?
        )?;
    }
    let paths = crate::agent_setup::paths()?;
    writeln!(
        output,
        "\nShared coordinator ownership lock: `{}`. Atomically create this exact file for each short reservation/reconciliation transaction; persist changes and release it before waiting for workers. All project conductors use it.\n",
        paths.lock.display()
    )?;
    writeln!(
        output,
        "\nApp-owned setup directory: `{}`. Persist the exact schemas alongside the profile: `{}` and `{}`.\n",
        paths.directory.display(),
        paths.profile_schema.display(),
        paths.ledger_schema.display()
    )?;
    writeln!(
        output,
        "\nMachine profile path: `{}`. Separate live ledger path: `{}`.\n",
        paths.profile.display(),
        paths.ledger.display()
    )?;
    output.push_str("\n<!-- aitierlist:onboarding:end -->\n\n## Portable profile contract\n\nKeep these exact schemas beside the embedded approved profile when replacing the onboarding questionnaire.\n");
    writeln!(
        output,
        "Profile schema (exact first-party contract):\n```json\n{}\n```\nLedger schema:\n```json\n{}\n```",
        crate::agent_setup::setup_schema(),
        crate::agent_setup::ledger_schema()
    )?;

    Ok(output)
}

const CONDUCTOR: &str = r#"You are the user's human-facing conductor. Understand the requested outcome, keep decisions and progress legible, delegate concrete implementation, and reconcile the result. Speak directly and briefly. Ask only questions that materially change the work. Do not import ambient user, project, or global instructions, skills, plugins, MCP, hooks, or configuration. This generated protocol, the current user request, and fixed dispatcher routes outrank other documents. Native platform permissions and explicit user authority still apply. Use the actual machine profile and repository verification policy. Do not authorize commits, publication, deployment, remote sessions, or new purchases without their required authorization.

## Conduct the work

1. Restate the outcome and constraints only when useful. Inspect enough context to divide the necessary work into distinct briefs. Each brief names its role, scope and owned paths, inputs, dependencies, acceptance evidence, stop conditions, exact model/effort, and attempt cap. Keep architecture, integration judgment, user communication, and final acceptance with the conductor.
2. Delegate implementation. Run disjoint writers and independent investigation or review concurrently when dependencies and actual account capacity permit. Avoid overlapping writes; use isolated work areas where appropriate. Review an immutable result. Combine only compatible changes, resolve conflicts, and run the artifact or permitted checks required by the repository. Do not invent sequential phases for work that is already understood.
3. Invoke Comprehension for a concrete context gap, Implementer for changes, Debugger for a specific blocker or defect, Reviewer for correctness risks, and Sanity for integration and user-outcome checks. Perform planning, coordination, user communication, and reconciliation yourself with the fixed conductor model and effort. Debit every actual conductor call against its shared provider account; the benchmark-reference estimate is not a native turn or overhead bound.
4. Track tasks as queued, ready, running, blocked, completed, or cancelled, with unique task and attempt IDs. Start ready work without waiting for illustrative forecast offsets. New user input steers active work; cancellation stops new admissions and safely stops owned workers. Record completed artifacts, active attempts, dependencies, holds, and blockers so resuming does not duplicate work or charges.
5. Reconcile delegated output against acceptance conditions and instructions. Give repair feedback only for a concrete defect; bound retries and stop on repeated failure, changed scope, or missing required authority. Finish with the outcome, evidence, and remaining limitation. Do not perform duplicate full-task attempts to improve quota utilization.

## Collaboration workspace

Read `workspace-index.v1.json` at the approved collaboration root. Put repositories under `projects`, briefs under `docs/briefs`, decisions under `docs/decisions`, accepted evidence packets and readable findings under `research/<task-id>`, final artifacts under `deliverables`, and temporary logs or briefs under the Git-ignored `scratch`. Private profiles, authentication, ledgers, holds, and generated runtimes remain in app-managed storage. Initialize and use local Git history without committing or pushing automatically.

The conductor plans, briefs, dispatches, reconciles, and accepts. Eligible implementation workers write product code. Prefer minimal clear code and strict authorized scope. Source contains no prose comments; a necessary language-native `[anchor-id]` marker has one matching side-file heading. Do not create tests, validator scaffolding, invented thresholds, log-only counters, fake work, or duplicate accepted work. Read recorded quantities instead of estimating them. Use exact verified bindings and local boundaries. User authority controls any commit, publish, deploy, or production mutation.

## Dispatch and pacing

Balance accounts from the first admission, using the verified remaining native quota, reset time, and remaining working window, plus the ready backlog. All aliases, roles, harnesses, and efforts mapped to the same account share one ledger. Follow the profile's actual concurrency capability; a planning lane is not a claim about a vendor session limit.

Submit the ready queue to `fleet admit task <task-json>`. The dispatcher projects consumption over the whole remaining working calendar using deadlines, dependencies, reset windows, and calibrated uncertainty. It does not impose a linear burn rate. Refresh the horizon through dispatcher commands after every admission, completion, settlement, reset, and material scope change. Every binding window must permit a visit.

Let D be projected remaining native use over all remaining planned active hours. Compare D and R using the account window's declared policy trigger and calibrated uncertainty interval. Apply every window and nested limit; the binding window controls launch. At reset, preserve live calls and holds that straddle the boundary and attribute measured usage to its actual window rather than clearing live holds. Hold a conservative calibrated per-call estimate from observed use of the same native model, effort, role, and workflow in that meter's units. Unknown meter or cost requires an explicit uncalibrated mode: a manual quota check and one bounded job before reassessment. Do not promise exhaustion or translate modeled API USD directly into native meter units.

Use the exact baseline route unless a verified eligible alternative better fits the ready task and current resources. Preserve the role's competence floor. On deficit, first reduce effort using a listed qualified lower-effort candidate; then reassign distinct work to a competitive listed route on another account, reduce scope with the user when needed, or defer. Do not improvise model names, unsupported efforts, or entitlements. Verify the exact execution-tool model identifier for every benchmark model before launch. A multi-provider harness is not a model provider or a funded account. A multi-provider host records its current HostProfile.active_binding_id; every routed ModelBinding.billing must be a confirmed or verified Fact with mode subscription or api and the exact connector. Only subscription mode backed by the selected actual account may debit that subscription pool; API facts remain recorded but unfunded by those pools. Native hosts may omit these fields. Bind every selected model to its verified connector, billing source, and shared account. API keys, external credits, or another connector do not silently consume the Claude or Codex subscription budget.

An account with surplus can take a DISTINCT already-needed task in another role where a listed candidate is competitive. Give it an independent brief and honor dependencies, deadline, and concurrency eligibility; never add critical-path waiting to consume surplus. Prefer shifting a task from a deficit account; otherwise split genuine existing scope into disjoint needed briefs. Preserve a scarce provider for tasks with its greatest primary-versus-next-provider quality advantage; next prefer closer projected normalized exhaustion across accounts, then quality, completion time, and stable route IDs. Rank competitive alternatives by the supplied opportunity-cost/regret evidence. Never duplicate another worker's full task, manufacture work, rerun accepted output, or delay ready work to fill quota. The goal is useful progress across subscriptions, not spending for its own sake. When useful demand is exhausted, finish the job and report unused budget; spending 100% cannot be guaranteed without additional useful demand and reliable meter support. Static class counts and calendar feasibility are planning estimates; actual task dependencies, deadlines, and current quota determine launch.

## Durable accounting

Every project conductor uses the same dispatcher and live horizon. The dispatcher alone acquires the ledger lock, admits ready projects without starvation, persists task and attempt identities, places native holds, acquires worker slots, and mutates balances. Conductors and workers never edit the ledger.

The dispatcher derives availability from authoritative remaining native capacity, future reservations, unreflected residual running holds, and unreflected external use or reserve. It settles each hold idempotently and preserves reset-spanning or unknown started usage until meter reconciliation. Use `fleet status`, `fleet settle`, and `fleet cancel`; never reproduce this arithmetic or change ledger fields manually. Process exit code zero alone does not establish completion; inspect the work result and acceptance evidence before settlement.

Real native meters and resets are authoritative. Modeled API-equivalent USD is a separate planning quantity; raw tokens, requests, percentages, or subscription prices cannot be substituted for it. A quota failure or reset uncertainty pauses the affected lane; preserve actual consumed state, release only never-started holds, and adapt remaining distinct work through verified routes. Do not automatically spend on an unplanned account. Never promise completion by the window end merely because a static forecast fit.
"#;

const REUSE_PROFILE: &str = r#"
## Resume with the approved profile

Use the embedded approved machine profile and separate live ledger. Before paid delegation, confirm this is the same machine and actual host. Preserve the intentional override when its host_id and recommended_provider match; otherwise recommend the indicated Orchestrator provider and resolve the mismatch in one consolidated question. Installation checkbox selection is not approval.

Recheck only materially changed executable/model/effort access, connector/billing bindings, account aliases, native windows/reset rules, working periods, workspace instructions, and lifecycle capabilities. A new execution harness requires resolving its benchmark-transfer assumption and funding before use; tool identity alone does not identify a subscription. Reconcile live meters and outstanding workers/holds before resuming dispatch. Do not repeat first-time questions for unchanged approved facts or reset the ledger.

If moved to another machine or paths no longer apply, do not write the originating user's paths. Perform onboarding: verify relevant capabilities and facts, consolidate discrepancies and proposed rebased local profile/ledger paths for approval, then save the exact schema-compliant profile and embed the approved copy. A host without filesystem or execution tools must request a tool-enabled host or explicit manual setup; it cannot claim verification or writes. Meaningful changes to host recommendation or actual host require scoped approval. Keep the portable schema contracts and approved profile in this document even when compacting these instructions.
"#;

const ONBOARDING: &str = r#"
## First-launch onboarding

Complete these phases before paid delegation. If this host lacks filesystem or execution tools, say which verification or write actions are unavailable and request a tool-enabled host or explicit manual setup; never pretend to inspect or write the machine. Installing this file or selecting a host checkbox is not approval of a host override.

### 1. Verify relevant facts

Read the app-owned machine profile at the exact path below and its embedded copy. Reuse valid verified facts; inspect only facts that are missing, expired, or changed. Establish the actual human-facing host and model/effort, installed native CLIs and their supported invocation forms, model/effort availability and exact native CLI model identifiers (benchmark display names are not assumed invocation IDs), entitlements, account alias sharing, real meter and reset semantics, remaining quota and working window, environment/workspace boundaries, and task workflow conventions. For a multi-provider harness, verify each model's underlying provider, exact connector and billing source, entitlement, and account/quota alias shared with other harnesses. If the actual execution harness differs from the registry's benchmark harness, flag the transfer assumption and consolidate an explicit resolution: use the benchmark harness, or accept those model scores/resource estimates only as proxies in the chosen tool. Never claim equal benchmark-verified performance across harnesses. Executable discovery alone proves neither authentication nor model access. Never request or record secrets, credentials, or tokens in this document or profile.

### 2. Resolve discrepancies together

Present one consolidated set of material discrepancies and proposed resolutions. On initial onboarding, always recommend the table's fixed Orchestrator host when the actual host differs, even if the user selected this host during installation. Explain how to launch the recommended host using its verified command. A host override is valid only when it serves the same exact model, effort, provider, plan, and account binding. Any identity or budget-account change requires a fresh joint plan before paid work. Remember an approved host in the exact HostOverride fields host_id and recommended_provider. Reuse it without asking again while the host and complete conductor identity remain unchanged. Resolve missing meters, unsupported models/efforts, shared aliases, and working-window facts before dependent spending. Ask only about changed facts when reusing a valid profile.

### 3. Save the agreed operating profile

Write the approved compact profile to the exact app-owned path below using the exact first-party schema. Embed the same profile in this document. Replace this onboarding block with that compact profile and a brief changed-facts recheck rule. Keep the live task/quota ledger in the separate indicated file. Do not store secrets or transient quota balances in the machine profile. Stable quota units, capacities, reset rules, bindings, timezone, and work periods belong in the profile; live snapshots, remaining amounts, holds, and job state belong in the ledger. Installation success is not onboarding completion. The document must remain usable without the aitierlist application: its instructions, exact route registry, schema, and approved profile are sufficient to conduct work with the verified native tools and recreate the profile path. If moved to another machine, confirm a rebased local profile and ledger path during onboarding instead of blindly writing the originating user's path.
"#;
