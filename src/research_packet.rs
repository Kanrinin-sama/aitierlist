use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::team_policy::{TaskRequest, Visit};
use crate::types::Seat;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct EvidencePacket {
    pub id: String,
    pub task_id: String,
    pub visit_id: String,
    pub artifact_path: String,
    pub producer_attempt_id: String,
    pub created_at: String,
    pub consuming_decision: String,
    pub claims: Vec<EvidenceClaim>,
    pub model_confidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct EvidenceClaim {
    pub id: String,
    pub claim: String,
    pub source: EvidenceSource,
    pub direct_support: DirectSupport,
    pub source_uncertainty: String,
    pub consuming_decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct EvidenceSource {
    pub id: String,
    pub url: String,
    pub publisher: String,
    pub retrieved_at: String,
    pub source_type: String,
    pub freshness_seconds: u64,
    pub provenance: SourceProvenance,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct DirectSupport {
    pub span: Option<String>,
    pub extraction: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct SourceProvenance {
    pub mode: SourceMode,
    pub snapshot_id: Option<String>,
    pub observed_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceMode {
    Live,
    Cached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct ArtifactPacketRef {
    pub id: String,
    pub path: String,
    pub digest: String,
    pub producer_attempt_id: String,
}

pub fn accept_packet(
    task: &TaskRequest,
    visit: &Visit,
    packet: &EvidencePacket,
    settled_at: &str,
) -> Result<ArtifactPacketRef> {
    ensure!(
        visit.seat == Seat::NetResearch,
        "Evidence packets are accepted only from Net Research visits"
    );
    ensure!(
        visit
            .criteria
            .iter()
            .any(|criterion| criterion == "source_retrieval"),
        "Research visit does not require source retrieval"
    );
    ensure!(
        visit
            .criteria
            .iter()
            .any(|criterion| criterion == "citation_accuracy"),
        "Research visit does not require citation evidence"
    );
    ensure!(
        packet.task_id == task.task_id && packet.visit_id == visit.id,
        "Evidence packet task or visit identity differs"
    );
    ensure!(
        !packet.id.trim().is_empty()
            && !packet.artifact_path.trim().is_empty()
            && !packet.producer_attempt_id.trim().is_empty(),
        "Evidence packet artifact identity is incomplete"
    );
    ensure!(
        !packet.consuming_decision.trim().is_empty(),
        "Evidence packet has no consuming decision"
    );
    ensure!(
        !packet.claims.is_empty(),
        "Evidence packet contains no claims"
    );
    let settled =
        OffsetDateTime::parse(settled_at, &Rfc3339).context("Settlement timestamp is invalid")?;
    let created = OffsetDateTime::parse(&packet.created_at, &Rfc3339)
        .context("Evidence packet timestamp is invalid")?;
    ensure!(
        created <= settled,
        "Evidence packet was created after settlement"
    );

    let mut ids = BTreeSet::new();
    ids.insert(packet.id.as_str());
    for claim in &packet.claims {
        ensure!(
            !claim.id.trim().is_empty() && ids.insert(&claim.id),
            "Evidence packet IDs are empty or duplicated"
        );
        ensure!(!claim.claim.trim().is_empty(), "Evidence claim is empty");
        ensure!(
            claim.consuming_decision == packet.consuming_decision,
            "Evidence claim is not tied to the packet's consuming decision"
        );
        ensure!(
            !claim.source_uncertainty.trim().is_empty(),
            "Evidence claim lacks source uncertainty"
        );
        ensure!(
            !claim.source.id.trim().is_empty() && ids.insert(&claim.source.id),
            "Evidence source IDs are empty or duplicated"
        );
        ensure!(
            url::Url::parse(&claim.source.url).is_ok(),
            "Evidence source URL is invalid"
        );
        ensure!(
            !claim.source.publisher.trim().is_empty()
                && !claim.source.source_type.trim().is_empty(),
            "Evidence source publisher or type is absent"
        );
        let retrieved = OffsetDateTime::parse(&claim.source.retrieved_at, &Rfc3339)
            .context("Source retrieval timestamp is invalid")?;
        let observed = OffsetDateTime::parse(&claim.source.provenance.observed_at, &Rfc3339)
            .context("Source provenance timestamp is invalid")?;
        ensure!(
            retrieved <= settled && observed <= retrieved,
            "Evidence source timestamps are inconsistent"
        );
        let age = (settled - retrieved).whole_seconds();
        ensure!(
            age >= 0 && u64::try_from(age).is_ok_and(|age| age <= claim.source.freshness_seconds),
            "Evidence source exceeds its declared freshness requirement"
        );
        match claim.source.provenance.mode {
            SourceMode::Live => ensure!(
                claim.source.provenance.snapshot_id.is_none(),
                "Live source provenance cannot name a cache snapshot"
            ),
            SourceMode::Cached => ensure!(
                claim
                    .source
                    .provenance
                    .snapshot_id
                    .as_ref()
                    .is_some_and(|id| !id.trim().is_empty()),
                "Cached source provenance lacks a snapshot ID"
            ),
        }
        ensure!(
            claim
                .direct_support
                .span
                .as_ref()
                .is_some_and(|span| !span.trim().is_empty())
                || !claim.direct_support.extraction.is_empty(),
            "Evidence claim lacks direct support"
        );
    }
    apply_task_requirements(task, packet)?;
    let bytes = serde_json::to_vec(packet)?;
    Ok(ArtifactPacketRef {
        id: packet.id.clone(),
        path: packet.artifact_path.clone(),
        digest: hex::encode(Sha256::digest(bytes)),
        producer_attempt_id: packet.producer_attempt_id.clone(),
    })
}

fn apply_task_requirements(task: &TaskRequest, packet: &EvidencePacket) -> Result<()> {
    for requirement in &task.evidence_requirements {
        let requirement =
            requirement
                .split_once(':')
                .map_or(requirement.as_str(), |(seat, value)| {
                    if seat.trim().eq_ignore_ascii_case(Seat::NetResearch.name()) {
                        value.trim()
                    } else {
                        ""
                    }
                });
        if requirement.is_empty()
            || matches!(
                requirement,
                "source_retrieval" | "citation_accuracy" | "known_source_fetch"
            )
        {
            continue;
        }
        if let Some(source_type) = requirement.strip_prefix("source_type=") {
            ensure!(
                packet
                    .claims
                    .iter()
                    .any(|claim| claim.source.source_type == source_type),
                "Evidence packet lacks required source type"
            );
        } else if let Some(publisher) = requirement.strip_prefix("publisher=") {
            ensure!(
                packet
                    .claims
                    .iter()
                    .any(|claim| claim.source.publisher == publisher),
                "Evidence packet lacks required publisher"
            );
        } else if let Some(decision) = requirement.strip_prefix("decision=") {
            ensure!(
                packet.consuming_decision == decision,
                "Evidence packet consuming decision differs from the task requirement"
            );
        } else if let Some(seconds) = requirement.strip_prefix("freshness_seconds=") {
            let seconds = seconds
                .parse::<u64>()
                .context("Research freshness requirement is invalid")?;
            ensure!(
                packet
                    .claims
                    .iter()
                    .all(|claim| claim.source.freshness_seconds <= seconds),
                "Evidence packet exceeds the task freshness requirement"
            );
        } else {
            ensure!(
                packet
                    .claims
                    .iter()
                    .any(|claim| claim.direct_support.extraction.contains_key(requirement)),
                "Evidence packet does not satisfy a declared research field"
            );
        }
    }
    Ok(())
}
