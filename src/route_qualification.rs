use anyhow::{Context, Result, ensure};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::agent_setup::{BillingMode, EvidenceConfidence, FactStatus, Ledger, SetupProfile};
use crate::isolation::Route;
use crate::team_policy::{TaskRequest, Visit};
use crate::types::Seat;

pub fn qualify<'a>(
    task: &TaskRequest,
    visit: &Visit,
    route: &'a Route,
    profile: &SetupProfile,
    ledger: &Ledger,
) -> Result<&'a Route> {
    ensure!(
        task.authorized && task.ready,
        "Task is not authorized and ready"
    );
    let planning_time = OffsetDateTime::parse(&ledger.updated_at, &Rfc3339)
        .context("Ledger planning timestamp is invalid")?;
    let binding = profile
        .bindings
        .iter()
        .find(|binding| binding.id == route.binding_id)
        .context("Route binding is absent from the current profile")?;
    let host = profile
        .hosts
        .iter()
        .find(|host| host.id == binding.host_id)
        .context("Route host is absent from the current profile")?;
    let account = profile
        .accounts
        .iter()
        .find(|account| account.id == binding.account_id)
        .context("Route account is absent from the current profile")?;

    ensure!(
        route.binding_id == binding.id,
        "Route binding identity differs"
    );
    ensure!(
        route.account_id == binding.account_id,
        "Route account identity differs"
    );
    ensure!(route.kind == host.kind, "Route host kind differs");
    ensure!(
        route.executable.to_string_lossy().as_ref()
            == approved_fact(&host.executable, "host executable")?,
        "Route executable differs"
    );
    ensure!(
        &route.model == approved_fact(&binding.native_model, "native model")?,
        "Route native model differs"
    );
    approved_fact(&binding.display_effort, "model effort provenance")?;
    approved_fact(&binding.effort_args, "native effort arguments")?;
    ensure!(
        verified_true(&host.installed) && verified_true(&binding.entitlement),
        "Route installation or entitlement is unverified"
    );
    ensure!(
        route.authenticated && verified_true(&account.authenticated),
        "Route authentication is unverified"
    );
    ensure!(
        account.provider_id == binding.provider_id,
        "Route provider and account differ"
    );
    ensure!(
        host.account_ids.contains(&account.id),
        "Route account is not approved for this host"
    );
    let provider = profile
        .providers
        .get(&binding.provider_id)
        .context("Provider binding is absent")?;
    ensure!(
        provider.account_id == account.id && provider.host_ids.contains(&host.id),
        "Provider binding does not match this route"
    );
    let billing = binding
        .billing
        .as_ref()
        .context("Exact route billing is unverified")?;
    let billing = approved_fact(billing, "exact route billing")?;
    ensure!(
        !billing.connector.trim().is_empty(),
        "Billing connector is absent"
    );
    ensure!(
        billing.mode == BillingMode::Subscription,
        "API billing cannot debit a selected subscription pool"
    );
    approved_fact(&account.plan, "subscription plan")?;

    let isolation = host
        .isolation
        .as_ref()
        .context("Route isolation is unverified")?;
    let isolation = approved_fact(isolation, "route isolation")?;
    ensure!(
        isolation.account_ids.contains(&account.id),
        "Isolation account differs"
    );
    ensure!(
        isolation.executable_identity == route.executable_identity,
        "Executable identity differs from the isolation recipe"
    );
    ensure!(
        route.home.starts_with(&isolation.generation_directory),
        "Route home is outside the approved generation"
    );
    ensure!(
        !isolation.executable_version.trim().is_empty(),
        "Executable version is unverified"
    );

    let launch_identity = crate::isolation::launch_identity(route)?;
    let bundle = profile
        .capability_bundles
        .iter()
        .filter(|bundle| bundle.binding_id == binding.id)
        .filter(|bundle| fresh(&bundle.observed_at, bundle.freshness_seconds, planning_time))
        .find(|bundle| bundle.launch_identity == launch_identity)
        .context("No fresh capability bundle matches the exact launch recipe")?;
    ensure!(
        route.permission == bundle.permission,
        "Compiled permission differs from the capability bundle"
    );
    ensure!(
        route.network_scope == bundle.network_scope,
        "Compiled network scope differs from the capability bundle"
    );
    ensure!(
        route.source_scope == bundle.source_scope,
        "Compiled source scope differs from the capability bundle"
    );
    ensure!(
        route.exclusions == bundle.exclusions,
        "Compiled exclusions differ from the capability bundle"
    );
    ensure!(bundle.isolated, "Route capability bundle is not isolated");
    ensure!(
        !bundle.permission.trim().is_empty(),
        "Route permission mode is unknown"
    );
    match_workflow(bundle, profile)?;
    match_required_resources(task, visit, bundle)?;
    if visit.seat == Seat::NetResearch {
        qualify_research(task, visit, bundle)?;
    }

    let evidence = profile
        .role_evidence
        .iter()
        .filter(|evidence| evidence.binding_id == binding.id && evidence.seat == visit.seat)
        .filter(|evidence| {
            fresh(
                &evidence.observed_at,
                evidence.freshness_seconds,
                planning_time,
            )
        })
        .find(|evidence| evidence_covers(task, visit, &evidence.dimensions))
        .context("No fresh role evidence covers this visit's criteria")?;
    match &evidence.confidence {
        EvidenceConfidence::Provisional => {
            ensure!(
                task.effective_risk().consequence < 2,
                "Provisional evidence cannot qualify consequential work"
            );
            ensure!(
                matches!(
                    task.effective_risk().template(),
                    crate::portfolio::WorkClass::Focused | crate::portfolio::WorkClass::Standard
                ),
                "Provisional evidence is limited to bounded workflows"
            );
            ensure!(
                !evidence.benchmark.trim().is_empty(),
                "Provisional evidence lacks a named benchmark"
            );
            ensure!(
                evidence
                    .transfer_assumption
                    .as_ref()
                    .is_some_and(|value| !value.trim().is_empty()),
                "Provisional evidence lacks a transfer rule"
            );
        }
        EvidenceConfidence::Calibrated => {
            let lower_bound = evidence
                .lower_bound
                .context("Calibrated evidence lacks a lower bound")?;
            let threshold = task
                .role_thresholds
                .get(visit.seat.name())
                .copied()
                .into_iter()
                .chain(
                    profile
                        .project_policies
                        .iter()
                        .find(|policy| policy.project_id == task.project_id)
                        .and_then(|policy| policy.role_thresholds.get(visit.seat.name()))
                        .copied(),
                )
                .chain(evidence.required_lower_bound)
                .reduce(f64::max)
                .context("Calibrated evidence lacks an approved role policy threshold")?;
            ensure!(
                lower_bound.is_finite() && threshold.is_finite() && lower_bound >= threshold,
                "Calibrated role lower bound is below policy"
            );
            ensure!(
                !evidence.benchmark.trim().is_empty() && !evidence.evidence.is_empty(),
                "Calibrated evidence lacks provenance"
            );
        }
    }
    if task.effective_risk().consequence >= 2 {
        ensure!(
            matches!(&evidence.confidence, EvidenceConfidence::Calibrated),
            "Consequential work requires calibrated role evidence"
        );
    }
    dependencies_accepted(task, ledger)?;
    Ok(route)
}

fn approved_fact<'a, T>(fact: &'a crate::agent_setup::Fact<T>, name: &str) -> Result<&'a T> {
    ensure!(fact.status == FactStatus::Verified, "{name} is unverified");
    fact.value
        .as_ref()
        .with_context(|| format!("{name} is absent"))
}

fn verified_true(fact: &crate::agent_setup::Fact<bool>) -> bool {
    (fact.status == FactStatus::Verified
        || fact.status == FactStatus::Confirmed && !fact.evidence.is_empty())
        && fact.value == Some(true)
}

fn fresh(observed_at: &str, freshness_seconds: u64, planning_time: OffsetDateTime) -> bool {
    OffsetDateTime::parse(observed_at, &Rfc3339).is_ok_and(|observed| {
        let age = (planning_time - observed).whole_seconds();
        age >= 0 && u64::try_from(age).is_ok_and(|age| age <= freshness_seconds)
    })
}

fn match_workflow(
    bundle: &crate::agent_setup::CapabilityBundle,
    profile: &SetupProfile,
) -> Result<()> {
    ensure!(
        bundle.permission == *approved_fact(&profile.workflow.approval, "workflow approval")?,
        "Capability permission differs from approved launch mode"
    );
    ensure!(
        !approved_fact(&profile.workflow.network, "workflow network")?
            .trim()
            .is_empty(),
        "Workflow network policy is empty"
    );
    ensure!(
        bundle.roots == *approved_fact(&profile.workflow.roots, "workflow roots")?,
        "Capability roots differ from approved launch mode"
    );
    ensure!(
        bundle.exclusions == *approved_fact(&profile.workflow.exclusions, "workflow exclusions")?,
        "Capability exclusions differ from approved launch mode"
    );
    let tools = approved_fact(&profile.workflow.tools, "workflow tools")?;
    ensure!(
        bundle.tools.iter().all(|tool| tools.contains(tool)),
        "Capability tools differ from approved launch mode"
    );
    Ok(())
}

fn match_required_resources(
    task: &TaskRequest,
    visit: &Visit,
    bundle: &crate::agent_setup::CapabilityBundle,
) -> Result<()> {
    for declared in &task.required_resources {
        let (scope, required) = declared
            .split_once('/')
            .unwrap_or(("global", declared.as_str()));
        ensure!(
            scope == "global"
                || crate::types::Seat::ALL
                    .iter()
                    .any(|seat| scope.eq_ignore_ascii_case(seat.name())),
            "Required resource has an unknown role scope"
        );
        if scope != "global" && !scope.eq_ignore_ascii_case(visit.seat.name()) {
            continue;
        }
        let (kind, value) = required
            .split_once(':')
            .context("Required resource lacks a type")?;
        let present = match kind {
            "tool" => bundle.tools.iter().any(|item| item == value),
            "network" => bundle.network_scope.iter().any(|item| item == value),
            "source" => bundle.source_scope.iter().any(|item| item == value),
            "root" => bundle.roots.iter().any(|item| item == value),
            "exclusion" => bundle.exclusions.iter().any(|item| item == value),
            "permission" => bundle.permission == value,
            _ => false,
        };
        ensure!(present, "Route lacks required resource {declared}");
    }
    Ok(())
}

fn qualify_research(
    task: &TaskRequest,
    visit: &Visit,
    bundle: &crate::agent_setup::CapabilityBundle,
) -> Result<()> {
    let has_fetch = bundle.tools.iter().any(|tool| {
        matches!(
            tool.to_ascii_lowercase().as_str(),
            "fetch" | "webfetch" | "web_fetch"
        )
    });
    let has_search = bundle.tools.iter().any(|tool| {
        matches!(
            tool.to_ascii_lowercase().as_str(),
            "search" | "websearch" | "web_search"
        )
    });
    let known_sources = task.evidence_requirements.iter().any(|requirement| {
        requirement == "known_source_fetch"
            || requirement
                .split_once(':')
                .is_some_and(|(seat, dimension)| {
                    seat.trim().eq_ignore_ascii_case(Seat::NetResearch.name())
                        && dimension.trim() == "known_source_fetch"
                })
    });
    ensure!(
        has_fetch && (known_sources || has_search),
        "Research tools do not cover the required retrieval mode"
    );
    ensure!(
        !bundle.source_scope.is_empty() && !bundle.network_scope.is_empty(),
        "Research source or network scope is absent"
    );
    ensure!(
        visit
            .criteria
            .iter()
            .any(|criterion| criterion == "citation_accuracy"),
        "Research visit lacks citation criteria"
    );
    Ok(())
}

fn evidence_covers(task: &TaskRequest, visit: &Visit, dimensions: &[String]) -> bool {
    let visit_criteria = visit
        .criteria
        .iter()
        .all(|criterion| dimensions.contains(criterion));
    let requested = task.evidence_requirements.iter().all(|requirement| {
        if let Some((seat, dimension)) = requirement.split_once(':') {
            let known_seat = Seat::ALL
                .iter()
                .any(|known| seat.trim().eq_ignore_ascii_case(known.name()));
            known_seat
                && (!seat.trim().eq_ignore_ascii_case(visit.seat.name())
                    || dimension.trim() == "known_source_fetch"
                    || dimensions.iter().any(|value| value == dimension.trim()))
        } else {
            requirement == "known_source_fetch"
                || dimensions.iter().any(|value| value == requirement)
        }
    });
    visit_criteria && requested
}

fn dependencies_accepted(task: &TaskRequest, ledger: &Ledger) -> Result<()> {
    for dependency in &task.dependencies {
        ensure!(
            ledger
                .jobs
                .iter()
                .flat_map(|job| &job.artifact_records)
                .any(|artifact| artifact.id == *dependency
                    && !artifact.digest.is_empty()
                    && artifact.accepted_at.is_some()),
            "Dependency lacks an accepted immutable artifact: {dependency}"
        );
    }
    Ok(())
}
