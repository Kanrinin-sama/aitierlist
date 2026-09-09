use std::collections::BTreeSet;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::agent_setup::{
    AttemptOutcome, Fact, FactStatus, ImmutableArtifact, JobStatus, Ledger, SetupProfile,
    UsageSettlement,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SettlementInput {
    pub idempotency_key: String,
    pub observed_at: String,
    pub recorded_at: String,
    pub attempt_id: Option<String>,
    pub external_conductor_hook: Option<String>,
    pub input_context_bucket: Option<String>,
    pub tool_calls: std::collections::BTreeMap<String, u64>,
    pub usage: Vec<SettledWindow>,
    pub outcome: AttemptOutcome,
    pub artifacts: Vec<ImmutableArtifact>,
    pub research_packet: Option<crate::research_packet::EvidencePacket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SettledWindow {
    pub hold_id: String,
    pub account_id: String,
    pub window_id: String,
    pub unit: String,
    pub amount: f64,
    pub attribution: String,
    pub provider_event_id: Option<String>,
    pub covered_by_observation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct MeterObservation {
    pub observation_id: String,
    pub account_id: String,
    pub window_id: String,
    pub unit: String,
    pub window_start: String,
    pub reset_at: String,
    pub observed_at: String,
    pub recorded_at: String,
    pub remaining: f64,
    pub evidence: Vec<String>,
    pub covered_hold_ids: Vec<String>,
    pub covered_external_reserve_ids: Vec<String>,
    pub covered_settlement_ids: Vec<String>,
    pub covered_provider_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct MeterObservationRecord {
    pub observation: MeterObservation,
    pub input_digest: String,
}

pub fn apply_settlement(
    profile: &SetupProfile,
    ledger: &mut Ledger,
    visit_id: &str,
    input: &SettlementInput,
) -> Result<Option<ImmutableArtifact>> {
    let digest = digest(input)?;
    let job_index = ledger
        .jobs
        .iter()
        .position(|job| job.id == visit_id)
        .context("Unknown visit ID")?;
    if let Some(existing) = ledger.jobs[job_index]
        .settlement_digests
        .get(&input.idempotency_key)
    {
        ensure!(
            existing == &digest,
            "Settlement idempotency key conflicts with prior content"
        );
        return Ok(input.research_packet.as_ref().and_then(|packet| {
            ledger.jobs[job_index]
                .artifact_records
                .iter()
                .find(|artifact| artifact.id == packet.id)
                .cloned()
        }));
    }
    ensure!(
        !input.idempotency_key.trim().is_empty(),
        "Settlement identity is empty"
    );
    let settled_at = parse(&input.observed_at, "settlement timestamp")?;
    ensure!(
        settled_at <= parse(&input.recorded_at, "settlement record timestamp")?,
        "Settlement timestamp follows its record time"
    );
    let job = &ledger.jobs[job_index];
    let resource_only = job.status == JobStatus::Completed;
    ensure!(
        matches!(
            job.status,
            JobStatus::Running | JobStatus::Held | JobStatus::Completed
        ),
        "Visit is not awaiting settlement or resource reconciliation"
    );
    if let Some(started_at) = &job.started_at {
        ensure!(
            settled_at >= parse(started_at, "attempt start timestamp")?,
            "Settlement predates the attempt start"
        );
    }
    if resource_only {
        ensure!(
            input.artifacts.is_empty()
                && input.research_packet.is_none()
                && job
                    .outcome
                    .as_ref()
                    .is_some_and(|outcome| serde_json::to_value(outcome).ok()
                        == serde_json::to_value(&input.outcome).ok()),
            "Resource reconciliation cannot replace an accepted outcome or artifacts"
        );
    }
    if input.attempt_id.is_some() {
        ensure!(
            job.status == JobStatus::Running || resource_only,
            "Worker settlement requires a running started attempt"
        );
    } else {
        ensure!(
            job.seat == Some(crate::types::Seat::Orchestrator)
                && input
                    .external_conductor_hook
                    .as_ref()
                    .is_some_and(|hook| !hook.trim().is_empty()),
            "Held settlement requires an external conductor hook"
        );
    }
    ensure!(
        input
            .input_context_bucket
            .as_ref()
            .is_none_or(|bucket| !bucket.trim().is_empty())
            && input.tool_calls.keys().all(|tool| !tool.trim().is_empty()),
        "Settlement telemetry descriptors are invalid"
    );
    let attempt_id = execution_identity(job, input)?;
    let hold_ids: BTreeSet<_> = job.holds.iter().map(|hold| hold.id.as_str()).collect();
    let usage_ids: BTreeSet<_> = input
        .usage
        .iter()
        .map(|usage| usage.hold_id.as_str())
        .collect();
    ensure!(
        usage_ids.len() == input.usage.len() && usage_ids == hold_ids,
        "Settlement must name every unique hold exactly once"
    );
    let event_ids: BTreeSet<_> = input
        .usage
        .iter()
        .filter_map(|usage| usage.provider_event_id.as_deref())
        .collect();
    ensure!(
        event_ids.len()
            == input
                .usage
                .iter()
                .filter(|usage| usage.provider_event_id.is_some())
                .count(),
        "Settlement provider event identities are duplicated"
    );

    let mut settlements = Vec::new();
    let mut resolved_holds = BTreeSet::new();
    for usage in &input.usage {
        ensure!(
            usage.amount.is_finite() && usage.amount >= 0.0 && !usage.attribution.trim().is_empty(),
            "Native usage settlement is invalid"
        );
        if let Some(event) = &usage.provider_event_id {
            ensure!(
                !ledger
                    .jobs
                    .iter()
                    .flat_map(|job| &job.actual_settled)
                    .any(|settlement| settlement.provider_event_id.as_ref() == Some(event)),
                "Provider event identity was already settled"
            );
        }
        let hold = job
            .holds
            .iter()
            .find(|hold| hold.id == usage.hold_id)
            .context("Settlement hold is absent")?;
        ensure!(
            hold.account_id == usage.account_id && hold.window_id == usage.window_id,
            "Settlement hold account or window differs"
        );
        let window = ledger
            .accounts
            .iter()
            .find(|account| account.account_id == usage.account_id)
            .and_then(|account| {
                account
                    .windows
                    .iter()
                    .find(|window| window.window_id == usage.window_id)
            })
            .context("Settlement window is absent")?;
        ensure!(
            window.unit == usage.unit,
            "Settlement unit differs from the native window"
        );
        let current_instance = hold.window_start.as_deref() == window.window_start.value.as_deref()
            && hold.reset_at.as_deref() == window.reset_at.value.as_deref();
        if current_instance && usage.covered_by_observation_id.is_none() {
            let snapshot_cutoff = window
                .snapshot
                .value
                .as_ref()
                .map(|snapshot| parse(&snapshot.observed_at, "snapshot cutoff"))
                .transpose()?;
            ensure!(
                snapshot_cutoff.is_none_or(|cutoff| settled_at >= cutoff),
                "Uncovered settlement predates the authoritative snapshot"
            );
        }
        profile_window(profile, &usage.account_id, &usage.window_id, &usage.unit)?;
        if let Some(observation_id) = &usage.covered_by_observation_id {
            let observation = ledger
                .meter_observations
                .iter()
                .find(|record| record.observation.observation_id == *observation_id)
                .context("Covering meter observation is absent")?;
            ensure!(
                observation.observation.account_id == usage.account_id
                    && observation.observation.window_id == usage.window_id
                    && observation.observation.unit == usage.unit
                    && hold.window_start.as_deref()
                        == Some(observation.observation.window_start.as_str())
                    && hold.reset_at.as_deref() == Some(observation.observation.reset_at.as_str()),
                "Covering observation belongs to another native window"
            );
            ensure!(
                parse(&observation.observation.observed_at, "observation cutoff")? >= settled_at,
                "Covering observation predates settlement"
            );
            ensure!(
                observation
                    .observation
                    .covered_hold_ids
                    .contains(&usage.hold_id)
                    || usage
                        .provider_event_id
                        .as_ref()
                        .is_some_and(|event| observation
                            .observation
                            .covered_provider_event_ids
                            .contains(event)),
                "Observation does not identify this settlement"
            );
        } else if usage.provider_event_id.is_some() && current_instance {
            let target = ledger
                .accounts
                .iter_mut()
                .find(|account| account.account_id == usage.account_id)
                .and_then(|account| {
                    account
                        .windows
                        .iter_mut()
                        .find(|window| window.window_id == usage.window_id)
                })
                .context("Settlement window is absent")?;
            target.spent_since_snapshot += usage.amount;
        }
        if usage.provider_event_id.is_some() || usage.covered_by_observation_id.is_some() {
            resolved_holds.insert(usage.hold_id.as_str());
        }
        settlements.push(UsageSettlement {
            settlement_id: input.idempotency_key.clone(),
            hold_id: usage.hold_id.clone(),
            account_id: usage.account_id.clone(),
            window_id: usage.window_id.clone(),
            unit: usage.unit.clone(),
            amount: usage.amount,
            attribution: usage.attribution.clone(),
            observed_at: input.observed_at.clone(),
            provider_event_id: usage.provider_event_id.clone(),
            window_start: hold.window_start.clone(),
            reset_at: hold.reset_at.clone(),
        });
    }

    let mut artifacts = input.artifacts.clone();
    let mut accepted_packet = None;
    if input.outcome.artifact_accepted && !resource_only {
        ensure!(
            !artifacts.is_empty() || job.seat == Some(crate::types::Seat::NetResearch),
            "Accepted outcome lacks an immutable artifact"
        );
        if job.seat == Some(crate::types::Seat::NetResearch) {
            let task = ledger
                .tasks
                .iter()
                .find(|task| task.task_id == job.parent_task_id)
                .context("Research task record is absent")?;
            let visit = crate::team_policy::visits(task)
                .into_iter()
                .find(|visit| visit.id == job.id)
                .context("Research visit definition is absent")?;
            let packet = input
                .research_packet
                .as_ref()
                .context("Accepted Net Research requires an evidence packet")?;
            let reference =
                crate::research_packet::accept_packet(task, &visit, packet, &input.observed_at)?;
            ensure!(
                reference.producer_attempt_id == attempt_id,
                "Research packet producer differs from the started attempt"
            );
            let artifact = ImmutableArtifact {
                id: reference.id,
                path: reference.path,
                digest: reference.digest,
                producer_attempt_id: reference.producer_attempt_id,
                parent_artifact_id: Some(artifacts.last().map_or_else(
                    || job.parent_task_id.clone(),
                    |artifact| artifact.id.clone(),
                )),
                accepted_at: Some(input.observed_at.clone()),
            };
            accepted_packet = Some(artifact.clone());
            artifacts.push(artifact);
        }
        artifact_lineage(
            ledger,
            &artifacts,
            &attempt_id,
            &job.parent_task_id,
            settled_at,
        )?;
    }
    let job = &mut ledger.jobs[job_index];
    job.holds
        .retain(|hold| !resolved_holds.contains(hold.id.as_str()));
    job.actual_settled.extend(settlements);
    if !resource_only {
        job.artifacts
            .extend(artifacts.iter().map(|artifact| artifact.id.clone()));
        job.artifact_records.extend(artifacts);
        job.outcome = Some(input.outcome.clone());
        job.status = if input.outcome.artifact_accepted {
            JobStatus::Completed
        } else {
            JobStatus::Blocked
        };
        job.remaining_seconds = Some(0.0);
        job.ended_at = Some(input.observed_at.clone());
        job.input_context_bucket = input.input_context_bucket.clone();
        job.tool_calls = input.tool_calls.clone();
    }
    job.settlement_keys.push(input.idempotency_key.clone());
    job.settlement_digests
        .insert(input.idempotency_key.clone(), digest);
    Ok(accepted_packet)
}

pub fn apply_observation(
    profile: &SetupProfile,
    ledger: &mut Ledger,
    input: &MeterObservation,
) -> Result<()> {
    let input_digest = digest(input)?;
    if let Some(existing) = ledger
        .meter_observations
        .iter()
        .find(|record| record.observation.observation_id == input.observation_id)
    {
        ensure!(
            existing.input_digest == input_digest,
            "Meter observation identity conflicts with prior content"
        );
        return Ok(());
    }
    ensure!(
        !input.observation_id.trim().is_empty()
            && input.remaining.is_finite()
            && input.remaining >= 0.0
            && !input.evidence.is_empty(),
        "Meter observation is incomplete"
    );
    let cutoff = parse(&input.observed_at, "meter observation cutoff")?;
    ensure!(
        cutoff <= parse(&input.recorded_at, "meter observation record timestamp")?,
        "Meter cutoff follows its record time"
    );
    let start = parse(&input.window_start, "meter window start")?;
    let reset = parse(&input.reset_at, "meter window reset")?;
    ensure!(
        start <= cutoff && cutoff < reset,
        "Meter cutoff is outside its reset instance"
    );
    profile_window(profile, &input.account_id, &input.window_id, &input.unit)?;
    let window = ledger
        .accounts
        .iter()
        .find(|account| account.account_id == input.account_id)
        .and_then(|account| {
            account
                .windows
                .iter()
                .find(|window| window.window_id == input.window_id)
        })
        .context("Observed native window is absent")?;
    ensure!(
        window.unit == input.unit,
        "Observed unit differs from the ledger window"
    );
    let same_instance = approved_time(&window.window_start, &input.window_start)
        && approved_time(&window.reset_at, &input.reset_at);
    let new_instance = window
        .reset_at
        .value
        .as_deref()
        .map(|value| parse(value, "previous window reset"))
        .transpose()?
        .is_some_and(|previous| start >= previous);
    ensure!(
        same_instance || new_instance,
        "Observation reset instance does not advance the ledger window"
    );
    if let Some((previous_cutoff, _)) = ledger
        .meter_observations
        .iter()
        .filter(|record| {
            record.observation.account_id == input.account_id
                && record.observation.window_id == input.window_id
                && record.observation.window_start == input.window_start
                && record.observation.reset_at == input.reset_at
        })
        .filter_map(|record| {
            parse(&record.observation.observed_at, "previous meter cutoff")
                .ok()
                .map(|instant| (instant, record))
        })
        .max_by(|left, right| left.0.cmp(&right.0))
    {
        ensure!(
            cutoff >= previous_cutoff,
            "Meter observation cutoff is not monotonic"
        );
    }
    let coverage = cumulative_coverage(ledger, input);
    unique_coverage(ledger, input)?;
    for job in &mut ledger.jobs {
        for hold in &mut job.holds {
            if coverage.holds.contains(&hold.id) {
                hold.reflected_through = Some(input.observed_at.clone());
            }
        }
    }
    for reserve in &mut ledger.external_reserves {
        if coverage.external.contains(&reserve.id) {
            reserve.reflected_through = Some(input.observed_at.clone());
        }
    }
    let unreflected: f64 = ledger
        .jobs
        .iter()
        .flat_map(|job| &job.actual_settled)
        .filter(|settlement| {
            settlement.account_id == input.account_id
                && settlement.window_id == input.window_id
                && settlement.unit == input.unit
                && settlement.provider_event_id.is_some()
                && settlement.window_start.as_deref() == Some(input.window_start.as_str())
                && settlement.reset_at.as_deref() == Some(input.reset_at.as_str())
        })
        .filter(|settlement| {
            !coverage.settlements.contains(&settlement.settlement_id)
                && settlement
                    .provider_event_id
                    .as_ref()
                    .is_none_or(|event| !coverage.events.contains(event))
        })
        .map(|settlement| settlement.amount)
        .sum();
    let window = ledger
        .accounts
        .iter_mut()
        .find(|account| account.account_id == input.account_id)
        .and_then(|account| {
            account
                .windows
                .iter_mut()
                .find(|window| window.window_id == input.window_id)
        })
        .context("Observed native window is absent")?;
    window.snapshot = Fact {
        status: FactStatus::Verified,
        value: Some(crate::agent_setup::MeterSnapshot {
            observed_at: input.observed_at.clone(),
            remaining: input.remaining,
            evidence: input.evidence.clone(),
        }),
        evidence: input.evidence.clone(),
    };
    window.spent_since_snapshot = unreflected;
    if new_instance {
        window.window_start = Fact {
            status: FactStatus::Verified,
            value: Some(input.window_start.clone()),
            evidence: input.evidence.clone(),
        };
        window.reset_at = Fact {
            status: FactStatus::Verified,
            value: Some(input.reset_at.clone()),
            evidence: input.evidence.clone(),
        };
    }
    ledger.meter_observations.push(MeterObservationRecord {
        observation: input.clone(),
        input_digest,
    });
    Ok(())
}

fn execution_identity(
    job: &crate::agent_setup::JobLedger,
    input: &SettlementInput,
) -> Result<String> {
    if let Some(attempt) = &input.attempt_id {
        ensure!(
            job.attempt_ids.contains(attempt),
            "Settlement attempt was not started for this visit"
        );
        return Ok(attempt.clone());
    }
    let hook = input
        .external_conductor_hook
        .as_ref()
        .context("Artifact acceptance requires a started attempt or external conductor hook")?;
    ensure!(
        job.seat == Some(crate::types::Seat::Orchestrator) && !hook.trim().is_empty(),
        "External execution hook is not valid for this visit"
    );
    Ok(hook.clone())
}

fn artifact_lineage(
    ledger: &Ledger,
    artifacts: &[ImmutableArtifact],
    producer: &str,
    task_id: &str,
    settled: OffsetDateTime,
) -> Result<()> {
    let accepted: BTreeSet<_> = ledger
        .jobs
        .iter()
        .flat_map(|job| &job.artifact_records)
        .filter(|artifact| artifact.accepted_at.is_some())
        .map(|artifact| artifact.id.as_str())
        .collect();
    for artifact in artifacts {
        ensure!(
            !artifact.id.trim().is_empty()
                && !artifact.path.trim().is_empty()
                && !artifact.digest.trim().is_empty(),
            "Immutable artifact identity is incomplete"
        );
        ensure!(
            artifact.producer_attempt_id == producer,
            "Artifact producer differs from the started attempt"
        );
        let parent = artifact
            .parent_artifact_id
            .as_ref()
            .context("Accepted artifact lacks a parent identity")?;
        ensure!(
            parent == task_id
                || accepted.contains(parent.as_str())
                || artifacts.iter().any(|candidate| candidate.id == *parent),
            "Artifact parent is not an accepted immutable artifact or task identity"
        );
        let accepted_at = artifact
            .accepted_at
            .as_ref()
            .context("Accepted artifact lacks an acceptance timestamp")?;
        ensure!(
            parse(accepted_at, "artifact acceptance timestamp")? <= settled,
            "Artifact acceptance timestamp follows settlement"
        );
    }
    Ok(())
}

struct Coverage {
    holds: BTreeSet<String>,
    external: BTreeSet<String>,
    settlements: BTreeSet<String>,
    events: BTreeSet<String>,
}

fn cumulative_coverage(ledger: &Ledger, input: &MeterObservation) -> Coverage {
    let records = ledger.meter_observations.iter().filter(|record| {
        record.observation.account_id == input.account_id
            && record.observation.window_id == input.window_id
            && record.observation.unit == input.unit
            && record.observation.window_start == input.window_start
            && record.observation.reset_at == input.reset_at
    });
    let mut coverage = Coverage {
        holds: input.covered_hold_ids.iter().cloned().collect(),
        external: input.covered_external_reserve_ids.iter().cloned().collect(),
        settlements: input.covered_settlement_ids.iter().cloned().collect(),
        events: input.covered_provider_event_ids.iter().cloned().collect(),
    };
    for record in records {
        coverage
            .holds
            .extend(record.observation.covered_hold_ids.iter().cloned());
        coverage.external.extend(
            record
                .observation
                .covered_external_reserve_ids
                .iter()
                .cloned(),
        );
        coverage
            .settlements
            .extend(record.observation.covered_settlement_ids.iter().cloned());
        coverage.events.extend(
            record
                .observation
                .covered_provider_event_ids
                .iter()
                .cloned(),
        );
    }
    coverage
}

fn unique_coverage(ledger: &Ledger, input: &MeterObservation) -> Result<()> {
    ensure!(
        unique(&input.covered_hold_ids)
            && unique(&input.covered_external_reserve_ids)
            && unique(&input.covered_settlement_ids)
            && unique(&input.covered_provider_event_ids),
        "Meter coverage identities are duplicated"
    );
    for id in &input.covered_hold_ids {
        let matches = ledger
            .jobs
            .iter()
            .flat_map(|job| &job.holds)
            .filter(|hold| {
                hold.id == *id
                    && hold.account_id == input.account_id
                    && hold.window_id == input.window_id
                    && hold.window_start.as_deref() == Some(input.window_start.as_str())
                    && hold.reset_at.as_deref() == Some(input.reset_at.as_str())
            })
            .count();
        ensure!(
            matches == 1,
            "Covered hold does not identify one hold in this native window"
        );
    }
    for id in &input.covered_external_reserve_ids {
        let matches = ledger
            .external_reserves
            .iter()
            .filter(|reserve| {
                reserve.id == *id
                    && reserve.account_id == input.account_id
                    && reserve.window_id == input.window_id
            })
            .count();
        ensure!(
            matches == 1,
            "Covered external reserve does not identify one reserve in this native window"
        );
    }
    for id in &input.covered_settlement_ids {
        ensure!(
            ledger
                .jobs
                .iter()
                .flat_map(|job| &job.actual_settled)
                .any(|settlement| settlement.settlement_id == *id
                    && settlement.account_id == input.account_id
                    && settlement.window_id == input.window_id
                    && settlement.unit == input.unit
                    && settlement.window_start.as_deref() == Some(input.window_start.as_str())
                    && settlement.reset_at.as_deref() == Some(input.reset_at.as_str())),
            "Covered settlement belongs to another native window or is absent"
        );
    }
    for id in &input.covered_provider_event_ids {
        ensure!(
            ledger
                .jobs
                .iter()
                .flat_map(|job| &job.actual_settled)
                .any(
                    |settlement| settlement.provider_event_id.as_ref() == Some(id)
                        && settlement.account_id == input.account_id
                        && settlement.window_id == input.window_id
                        && settlement.unit == input.unit
                        && settlement.window_start.as_deref() == Some(input.window_start.as_str())
                        && settlement.reset_at.as_deref() == Some(input.reset_at.as_str())
                ),
            "Covered provider event belongs to another native window or is absent"
        );
    }
    Ok(())
}

fn profile_window(
    profile: &SetupProfile,
    account_id: &str,
    window_id: &str,
    unit: &str,
) -> Result<()> {
    let window = profile
        .accounts
        .iter()
        .find(|account| account.id == account_id)
        .and_then(|account| {
            account
                .quota_windows
                .iter()
                .find(|window| window.id == window_id)
        })
        .context("Native profile window is absent")?;
    ensure!(
        window.unit.value.as_deref() == Some(unit) && window.unit.status != FactStatus::Unknown,
        "Native profile unit differs or is unverified"
    );
    Ok(())
}

fn approved_time(fact: &Fact<String>, expected: &str) -> bool {
    fact.status != FactStatus::Unknown && fact.value.as_deref() == Some(expected)
}

fn unique(values: &[String]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn parse(value: &str, name: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).with_context(|| format!("{name} is invalid"))
}

fn digest(value: &impl Serialize) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(value)?)))
}
