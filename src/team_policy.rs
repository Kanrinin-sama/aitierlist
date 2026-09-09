use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::agent_setup::NativeCoefficient;
use crate::portfolio::WorkClass;
use crate::types::Seat;

pub const VERSION: &str = concat!("aitierlist-", env!("CARGO_PKG_VERSION"), "-team-policy-v2");

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "snake_case")]
pub struct RiskVector {
    pub consequence: u8,
    pub uncertainty: u8,
    pub coupling: u8,
    pub reversibility: u8,
    pub evidence_need: u8,
    pub tool_risk: u8,
    pub correlation: u8,
    pub deadline: u8,
}

impl RiskVector {
    pub fn template(&self) -> WorkClass {
        let maximum = [
            self.consequence,
            self.uncertainty,
            self.coupling,
            self.reversibility,
            self.tool_risk,
            self.correlation,
            self.deadline,
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        if maximum >= 3 || self.consequence >= 2 && self.reversibility >= 2 {
            WorkClass::Extensive
        } else if maximum >= 2 || self.uncertainty + self.coupling >= 3 {
            WorkClass::Complex
        } else if maximum >= 1 {
            WorkClass::Standard
        } else {
            WorkClass::Focused
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct TaskRequest {
    pub task_id: String,
    pub project_id: String,
    pub generation_id: String,
    pub workspace_id: String,
    pub idempotency_key: String,
    pub brief_path: std::path::PathBuf,
    pub timeout_seconds: u64,
    pub authorized: bool,
    pub ready: bool,
    pub risk: RiskVector,
    pub evidence_requirements: Vec<String>,
    pub dependencies: Vec<String>,
    pub priority: u32,
    pub entitlement_weight: f64,
    pub deadline: Option<String>,
    pub ready_at: Option<String>,
    pub artifact_value: f64,
    pub required_resources: Vec<String>,
    pub human_seconds: f64,
    pub role_thresholds: BTreeMap<String, f64>,
}

impl TaskRequest {
    pub fn effective_risk(&self) -> RiskVector {
        let mut risk = self.risk.clone();
        if !self.evidence_requirements.is_empty() {
            risk.evidence_need = risk.evidence_need.max(1);
        }
        for resource in &self.required_resources {
            let value = resource.to_ascii_lowercase();
            if [
                "auth",
                "billing",
                "deletion",
                "production",
                "credentials",
                "deployment",
            ]
            .iter()
            .any(|trigger| value.contains(trigger))
            {
                risk.consequence = risk.consequence.max(2);
                risk.tool_risk = risk.tool_risk.max(2);
                risk.reversibility = risk.reversibility.max(2);
            }
            if ["network", "shell", "browser"]
                .iter()
                .any(|trigger| value.contains(trigger))
            {
                risk.tool_risk = risk.tool_risk.max(1);
            }
        }
        risk
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Visit {
    pub id: String,
    pub seat: Seat,
    pub ordinal: u8,
    pub dependencies: Vec<String>,
    pub conditional: bool,
    pub criteria: Vec<String>,
}

pub fn visits(task: &TaskRequest) -> Vec<Visit> {
    let risk = task.effective_risk();
    let template = risk.template();
    let research = risk.evidence_need > 0;
    let comprehension = matches!(template, WorkClass::Complex | WorkClass::Extensive);
    let checks = !matches!(template, WorkClass::Focused);
    let mut result = Vec::new();
    let mut add = |seat, ordinal, dependencies: Vec<String>, conditional, criteria: &[&str]| {
        let id = format!("{}-{}-{}", task.task_id, seat_id(seat), ordinal);
        result.push(Visit {
            id,
            seat,
            ordinal,
            dependencies,
            conditional,
            criteria: criteria.iter().map(|value| (*value).to_owned()).collect(),
        });
    };
    add(
        Seat::Orchestrator,
        1,
        Vec::new(),
        false,
        &["scope", "risk", "acceptance"],
    );
    if research {
        add(
            Seat::NetResearch,
            1,
            vec![format!("{}-orchestrator-1", task.task_id)],
            false,
            &["source_retrieval", "citation_accuracy"],
        );
    }
    if comprehension {
        let dependency = if research {
            "net-research"
        } else {
            "orchestrator"
        };
        add(
            Seat::Comprehension,
            1,
            vec![format!("{}-{dependency}-1", task.task_id)],
            false,
            &["repository_comprehension"],
        );
    }
    let implementation_dependency = if comprehension {
        "comprehension"
    } else if research {
        "net-research"
    } else {
        "orchestrator"
    };
    add(
        Seat::Implementer,
        1,
        vec![format!("{}-{implementation_dependency}-1", task.task_id)],
        false,
        &["patch_generation", "terminal_execution"],
    );
    if checks {
        add(
            Seat::Reviewer,
            1,
            vec![format!("{}-implementer-1", task.task_id)],
            false,
            &["adversarial_review"],
        );
        add(
            Seat::Sanity,
            1,
            vec![format!("{}-implementer-1", task.task_id)],
            false,
            &["acceptance_execution"],
        );
        add(
            Seat::Debugger,
            1,
            vec![format!("{}-implementer-1", task.task_id)],
            true,
            &["fault_localization", "repair"],
        );
        add(
            Seat::Reviewer,
            2,
            vec![format!("{}-debugger-1", task.task_id)],
            true,
            &["repair_review"],
        );
        add(
            Seat::Sanity,
            2,
            vec![format!("{}-debugger-1", task.task_id)],
            true,
            &["repair_acceptance_execution"],
        );
    }
    let final_dependencies = if checks {
        vec![
            format!("{}-reviewer-1", task.task_id),
            format!("{}-sanity-1", task.task_id),
        ]
    } else {
        vec![format!("{}-implementer-1", task.task_id)]
    };
    add(
        Seat::Orchestrator,
        2,
        final_dependencies,
        false,
        &["artifact_reconciliation"],
    );
    result
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Assignment {
    pub visit: Visit,
    pub route_id: String,
    pub binding_id: String,
    pub account_id: String,
    pub coefficients: Vec<NativeCoefficient>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NativePlan {
    pub policy_version: String,
    pub task_id: String,
    pub project_id: String,
    pub workflow: WorkClass,
    pub conductor_binding_id: String,
    pub assignments: Vec<Assignment>,
}

pub fn route_id(binding_id: &str) -> String {
    format!("route-{}", binding_id.trim_start_matches("model-"))
}

pub fn runtime_route_id(binding_id: &str, launch_identity: &str) -> String {
    format!(
        "{}-{}",
        route_id(binding_id),
        &launch_identity[..launch_identity.len().min(12)]
    )
}
