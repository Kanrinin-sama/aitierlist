use crate::types::{Row, Seat, Table};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKind {
    Codex,
    Claude,
    Muse,
    Gemini,
    Antigravity,
    Grok,
    #[serde(rename = "opencode")]
    OpenCode,
    Aider,
    Goose,
    Cline,
    #[serde(rename = "openhands")]
    OpenHands,
    Buzz,
    Pi,
    Droid,
}
impl HostKind {
    pub fn native_provider(self) -> Option<&'static str> {
        Some(match self {
            Self::Codex => "openai",
            Self::Claude => "anthropic",
            Self::Muse => "muse",
            Self::Gemini | Self::Antigravity => "google",
            Self::Grok => "xai",
            _ => return None,
        })
    }
    pub fn is_multi_provider(self) -> bool {
        matches!(
            self,
            Self::OpenCode
                | Self::Aider
                | Self::Goose
                | Self::Cline
                | Self::OpenHands
                | Self::Buzz
                | Self::Pi
                | Self::Droid
        )
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::Claude => "Claude",
            Self::Muse => "Muse",
            Self::Gemini => "Gemini",
            Self::Antigravity => "Antigravity",
            Self::Grok => "Grok",
            Self::OpenCode => "OpenCode",
            Self::Aider => "Aider",
            Self::Goose => "Goose",
            Self::Cline => "Cline",
            Self::OpenHands => "OpenHands",
            Self::Buzz => "Buzz Desktop",
            Self::Pi => "Pi",
            Self::Droid => "Droid",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FactStatus {
    Verified,
    Confirmed,
    Unknown,
}
fn required_nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> std::result::Result<Option<T>, D::Error> {
    Option::deserialize(deserializer)
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "T: Deserialize<'de>"))]
pub struct Fact<T> {
    pub status: FactStatus,
    #[serde(deserialize_with = "required_nullable")]
    pub value: Option<T>,
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Copy)]
pub struct Version1;
impl Serialize for Version1 {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_u8(1)
    }
}
impl<'de> Deserialize<'de> for Version1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        if u8::deserialize(deserializer)? == 1 {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom(
                "Only schema version 1 is supported",
            ))
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerProfile {
    pub id: String,
    pub hostname: Fact<String>,
    pub platform: Fact<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostProfile {
    pub id: String,
    pub kind: HostKind,
    pub installed: Fact<bool>,
    pub executable: Fact<String>,
    pub argv: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub account_ids: Vec<String>,
    pub background: Fact<bool>,
    pub lifecycle: Fact<String>,
    #[serde(default)]
    pub active_binding_id: Option<Fact<String>>,
    #[serde(default)]
    pub isolation: Option<Fact<IsolationBinding>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IsolationBinding {
    pub generation_directory: String,
    pub account_ids: Vec<String>,
    pub executable_identity: String,
    pub executable_version: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterCommand {
    pub executable: String,
    pub argv: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub read_only: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaWindow {
    pub id: String,
    pub unit: Fact<String>,
    pub capacity: Fact<f64>,
    pub reset_rule: Fact<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub parent_window: Option<String>,
    pub binding_ids: Vec<String>,
    pub meter: Fact<MeterCommand>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountProfile {
    pub id: String,
    pub provider_id: String,
    pub plan: Fact<String>,
    pub authenticated: Fact<bool>,
    pub quota_windows: Vec<QuotaWindow>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderBinding {
    pub account_id: String,
    pub host_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelBinding {
    pub id: String,
    pub provider_id: String,
    pub host_id: String,
    pub account_id: String,
    pub display_model: String,
    pub display_effort: Fact<String>,
    pub native_model: Fact<String>,
    pub model_args: Fact<Vec<String>>,
    pub effort_args: Fact<Vec<String>>,
    pub entitlement: Fact<bool>,
    #[serde(default)]
    pub billing: Option<Fact<BillingBinding>>,
}
impl ModelBinding {
    pub fn subscription_funded(&self, required: bool) -> bool {
        if self.entitlement.status == FactStatus::Unknown || self.entitlement.value != Some(true) {
            return false;
        }
        match &self.billing {
            None => !required,
            Some(fact) => fact.value.as_ref().is_some_and(|billing| {
                fact.status != FactStatus::Unknown
                    && billing.mode == BillingMode::Subscription
                    && !billing.connector.trim().is_empty()
            }),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingMode {
    Subscription,
    Api,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BillingBinding {
    pub mode: BillingMode,
    pub connector: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkPeriod {
    pub weekday: u8,
    pub start: String,
    pub end: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveWindow {
    pub start: String,
    pub end: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostOverride {
    pub host_id: String,
    pub recommended_provider: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowProfile {
    pub timezone: Fact<String>,
    pub work_periods: Fact<Vec<WorkPeriod>>,
    pub network: Fact<String>,
    pub instructions: Fact<Vec<String>>,
    pub exclusive_resources: Fact<Vec<String>>,
    pub roots: Fact<Vec<String>>,
    pub exclusions: Fact<Vec<String>>,
    pub tools: Fact<Vec<String>>,
    pub background: Fact<bool>,
    pub concurrency: Fact<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestrators: Option<Fact<usize>>,
    pub approval: Fact<String>,
    pub checks: Fact<Vec<String>>,
    pub available_hours: Fact<f64>,
    #[serde(default)]
    pub active_windows: Option<Fact<Vec<ActiveWindow>>>,
    #[serde(default)]
    pub human_available_seconds: Option<Fact<f64>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupProfile {
    pub version: Version1,
    pub computer: ComputerProfile,
    pub hosts: Vec<HostProfile>,
    pub accounts: Vec<AccountProfile>,
    pub providers: BTreeMap<String, ProviderBinding>,
    pub bindings: Vec<ModelBinding>,
    pub workflow: WorkflowProfile,
    pub orchestrator_host_override: Fact<HostOverride>,
    pub orchestrator_overhead_account: Fact<String>,
    #[serde(default)]
    pub native_coefficients: Vec<NativeCoefficient>,
    #[serde(default)]
    pub capability_bundles: Vec<CapabilityBundle>,
    #[serde(default)]
    pub role_evidence: Vec<RoleEvidence>,
    #[serde(default)]
    pub project_policies: Vec<ProjectPolicy>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPolicy {
    pub project_id: String,
    pub priority: u32,
    pub entitlement_weight: f64,
    pub minimum_workflow: crate::portfolio::WorkClass,
    pub role_thresholds: BTreeMap<String, f64>,
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityBundle {
    pub id: String,
    pub binding_id: String,
    pub tools: Vec<String>,
    pub permission: String,
    pub network_scope: Vec<String>,
    pub source_scope: Vec<String>,
    pub roots: Vec<String>,
    pub exclusions: Vec<String>,
    pub isolated: bool,
    pub observed_at: String,
    pub freshness_seconds: u64,
    pub evidence: Vec<String>,
    pub launch_identity: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceConfidence {
    Provisional,
    Calibrated,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleEvidence {
    pub id: String,
    pub binding_id: String,
    pub seat: Seat,
    pub dimensions: Vec<String>,
    pub confidence: EvidenceConfidence,
    pub benchmark: String,
    pub transfer_assumption: Option<String>,
    pub lower_bound: Option<f64>,
    #[serde(default)]
    pub required_lower_bound: Option<f64>,
    pub observed_at: String,
    pub freshness_seconds: u64,
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeCoefficient {
    pub id: String,
    pub binding_id: String,
    pub seat: Seat,
    pub workflow: crate::portfolio::WorkClass,
    pub window_id: String,
    pub account_id: String,
    pub unit: String,
    pub expected: f64,
    pub reserve: f64,
    pub expected_seconds: f64,
    pub reserve_seconds: f64,
    pub uncertainty_lower: f64,
    pub uncertainty_upper: f64,
    pub manual_pilot: bool,
    pub sample_count: u64,
    pub observed_from: String,
    pub observed_to: String,
    pub estimator: String,
    pub uncertainty: String,
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeterSnapshot {
    pub observed_at: String,
    pub remaining: f64,
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowLedger {
    pub window_id: String,
    pub unit: String,
    pub window_start: Fact<String>,
    pub reset_at: Fact<String>,
    pub snapshot: Fact<MeterSnapshot>,
    pub spent_since_snapshot: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountLedger {
    pub account_id: String,
    pub windows: Vec<WindowLedger>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hold {
    #[serde(default)]
    pub id: String,
    pub account_id: String,
    pub window_id: String,
    pub amount: f64,
    #[serde(default)]
    pub expected: f64,
    #[serde(default)]
    pub coefficient_id: String,
    #[serde(default)]
    pub reflected_through: Option<String>,
    #[serde(default)]
    pub window_start: Option<String>,
    #[serde(default)]
    pub reset_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobLedger {
    pub id: String,
    pub idempotency_key: String,
    pub binding_id: String,
    #[serde(default)]
    pub route_id: String,
    pub status: JobStatus,
    pub dependencies: Vec<String>,
    pub attempt_ids: Vec<String>,
    pub worker_handle: Fact<String>,
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub artifact_records: Vec<ImmutableArtifact>,
    pub recovery: Fact<String>,
    pub holds: Vec<Hold>,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub parent_task_id: String,
    #[serde(default)]
    pub workflow: Option<crate::portfolio::WorkClass>,
    #[serde(default)]
    pub seat: Option<Seat>,
    #[serde(default)]
    pub risk: Option<crate::team_policy::RiskVector>,
    #[serde(default)]
    pub evidence_requirements: Vec<String>,
    #[serde(default)]
    pub conditional: bool,
    #[serde(default)]
    pub actual_settled: Vec<UsageSettlement>,
    #[serde(default)]
    pub outcome: Option<AttemptOutcome>,
    #[serde(default)]
    pub scheduled_start: Option<String>,
    #[serde(default)]
    pub scheduled_end: Option<String>,
    #[serde(default)]
    pub remaining_seconds: Option<f64>,
    #[serde(default)]
    pub settlement_keys: Vec<String>,
    #[serde(default)]
    pub settlement_digests: BTreeMap<String, String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
    #[serde(default)]
    pub input_context_bucket: Option<String>,
    #[serde(default)]
    pub tool_calls: BTreeMap<String, u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImmutableArtifact {
    pub id: String,
    pub path: String,
    pub digest: String,
    pub producer_attempt_id: String,
    pub parent_artifact_id: Option<String>,
    pub accepted_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageSettlement {
    #[serde(default)]
    pub settlement_id: String,
    pub hold_id: String,
    #[serde(default)]
    pub account_id: String,
    pub window_id: String,
    #[serde(default)]
    pub unit: String,
    pub amount: f64,
    pub attribution: String,
    pub observed_at: String,
    pub provider_event_id: Option<String>,
    #[serde(default)]
    pub window_start: Option<String>,
    #[serde(default)]
    pub reset_at: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptOutcome {
    pub category: String,
    pub artifact_accepted: bool,
    pub rejection_reason: Option<String>,
    pub rework_cause: Option<String>,
    pub human_escalation: bool,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Ready,
    Blocked,
    Cancelled,
    Held,
    Running,
    Completed,
    Released,
    Deferred,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub version: Version1,
    pub computer_id: String,
    pub updated_at: String,
    pub accounts: Vec<AccountLedger>,
    pub jobs: Vec<JobLedger>,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub external_reserves: Vec<ExternalReserve>,
    #[serde(default)]
    pub current_horizon: Option<HorizonPointer>,
    #[serde(default)]
    pub tasks: Vec<crate::team_policy::TaskRequest>,
    #[serde(default)]
    pub horizons: Vec<HorizonPointer>,
    #[serde(default)]
    pub meter_observations: Vec<crate::native_reconciliation::MeterObservationRecord>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HorizonPointer {
    pub id: String,
    pub policy_version: String,
    pub profile_identity: String,
    pub committed_revision: u64,
    pub artifact_path: String,
    pub generation_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalReserve {
    pub id: String,
    pub account_id: String,
    pub window_id: String,
    pub amount: f64,
    pub reflected_through: Option<String>,
    pub evidence: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupPaths {
    pub lock: PathBuf,
    pub directory: PathBuf,
    pub profile: PathBuf,
    pub ledger: PathBuf,
    pub profile_schema: PathBuf,
    pub ledger_schema: PathBuf,
    pub generations: PathBuf,
    pub bin: PathBuf,
}
#[derive(Debug, Clone)]
pub struct SetupState {
    pub profile: Option<SetupProfile>,
    pub ready: bool,
    pub message: String,
    pub issues: Vec<String>,
}
pub fn paths() -> Result<SetupPaths> {
    let directory = crate::update::install_config()?
        .data_dir
        .join("orchestrator");
    Ok(SetupPaths {
        lock: directory.join("ledger.v1.lock"),
        profile: directory.join("setup.v1.json"),
        ledger: directory.join("ledger.v1.json"),
        profile_schema: directory.join("setup.v1.schema.json"),
        ledger_schema: directory.join("ledger.v1.schema.json"),
        generations: directory.join("generations"),
        bin: directory.join("bin"),
        directory,
    })
}
pub fn binding_id(row: &Row) -> String {
    binding_id_for(&row.harness, row)
}
pub fn binding_id_for(harness: &str, row: &Row) -> String {
    format!(
        "model-{}",
        &hex::encode(Sha256::digest(
            format!("{}\0{}\0{:?}", harness, row.model_key, row.effort).as_bytes()
        ))[..24]
    )
}
pub(crate) fn computer_id() -> String {
    format!(
        "computer-{}",
        &hex::encode(Sha256::digest(
            format!(
                "{}:{}",
                std::env::consts::OS,
                std::env::var("COMPUTERNAME")
                    .or_else(|_| std::env::var("HOSTNAME"))
                    .unwrap_or_default()
            )
            .as_bytes()
        ))[..24]
    )
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(2_097_153)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 2_097_152 {
        bail!("{} exceeds the 2 MiB profile limit", path.display());
    }
    serde_json::from_slice(&bytes).with_context(|| format!("Invalid {}", path.display()))
}

pub fn profile_identity(profile: &SetupProfile) -> Result<String> {
    Ok(hex::encode(Sha256::digest(serde_json::to_vec(profile)?)))
}

struct LedgerLock {
    _file: std::fs::File,
}

fn ledger_lock(path: &Path) -> Result<LedgerLock> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path)
            .context("The shared ledger is busy; retry this dispatch transaction")?;
        Ok(LedgerLock { _file: file })
    }
    #[cfg(not(windows))]
    {
        Ok(LedgerLock {
            _file: OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(path)?,
        })
    }
}

fn read_runtime_unlocked(paths: &SetupPaths) -> Result<(SetupProfile, Ledger, String)> {
    let profile: SetupProfile = read_json(&paths.profile)?;
    check_profile(&profile)?;
    let ledger: Ledger = read_json(&paths.ledger)?;
    check_ledger(&ledger, &profile)?;
    let identity = profile_identity(&profile)?;
    Ok((profile, ledger, identity))
}

pub fn read_runtime(paths: &SetupPaths) -> Result<(SetupProfile, Ledger, String)> {
    let lock = ledger_lock(&paths.lock)?;
    let result = read_runtime_unlocked(paths);
    drop(lock);
    result
}

pub fn transact<T>(
    paths: &SetupPaths,
    expected_profile_identity: &str,
    expected_revision: Option<u64>,
    change: impl FnOnce(&SetupProfile, &mut Ledger) -> Result<T>,
) -> Result<T> {
    let _lock = ledger_lock(&paths.lock)?;
    let (profile, mut ledger, identity) = read_runtime_unlocked(paths)?;
    ensure!(
        identity == expected_profile_identity,
        "Capability profile changed; compile and solve a new fleet policy"
    );
    if let Some(revision) = expected_revision {
        ensure!(
            ledger.revision == revision,
            "Ledger changed; solve the current horizon again"
        );
    }
    let result = change(&profile, &mut ledger)?;
    ledger.revision = ledger
        .revision
        .checked_add(1)
        .context("Ledger revision exhausted")?;
    ledger.updated_at =
        time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339)?;
    crate::file_tx::replace_file_contents(&paths.ledger, &serde_json::to_vec_pretty(&ledger)?)?;
    Ok(result)
}
fn unique<'a>(values: impl Iterator<Item = &'a str>, label: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !safe_id(value) || !seen.insert(value) {
            bail!("Empty or duplicate {label}: {value}");
        }
    }
    Ok(())
}
fn safe_id(value: &str) -> bool {
    value.len() <= 96
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
fn environment(values: &BTreeMap<String, String>) -> Result<()> {
    for (key, value) in values {
        if ![
            "CODEX_HOME",
            "CLAUDE_CONFIG_DIR",
            "GROK_HOME",
            "GEMINI_CLI_HOME",
        ]
        .contains(&key.as_str())
            || !Path::new(value).is_absolute()
        {
            bail!(
                "Unsupported host environment entry {key}; only absolute config-home paths are accepted"
            );
        }
    }
    Ok(())
}
fn facts(value: &Value) -> Result<()> {
    if let Some(object) = value.as_object() {
        if let Some(status) = object
            .get("status")
            .and_then(Value::as_str)
            .filter(|status| ["verified", "confirmed", "unknown"].contains(status))
        {
            let known = status != "unknown";
            if known
                && object["value"]
                    .as_str()
                    .is_some_and(|text| text.trim().is_empty())
            {
                bail!(
                    "Known string facts must be nonempty; use an explicit none value when applicable"
                );
            }
            if known == object["value"].is_null()
                || (known && object["evidence"].as_array().is_none_or(Vec::is_empty))
            {
                bail!(
                    "Facts require null for unknown, and a value plus evidence for verified/confirmed"
                );
            }
        }
        for child in object.values() {
            facts(child)?;
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            facts(child)?;
        }
    }
    Ok(())
}
fn timestamp(value: &str) -> Result<time::OffsetDateTime> {
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .context("Expected RFC3339 timestamp")
}
fn check_ledger(ledger: &Ledger, profile: &SetupProfile) -> Result<()> {
    if ledger.computer_id != profile.computer.id {
        bail!("Ledger belongs to a different computer");
    }
    facts(&serde_json::to_value(ledger)?)?;
    timestamp(&ledger.updated_at)?;
    unique(
        ledger
            .accounts
            .iter()
            .map(|account| account.account_id.as_str()),
        "ledger account ID",
    )?;
    unique(
        ledger
            .external_reserves
            .iter()
            .map(|reserve| reserve.id.as_str()),
        "external reserve ID",
    )?;
    unique(ledger.jobs.iter().map(|job| job.id.as_str()), "job ID")?;
    unique(
        ledger.tasks.iter().map(|task| task.task_id.as_str()),
        "task ID",
    )?;
    unique(
        ledger
            .tasks
            .iter()
            .map(|task| task.idempotency_key.as_str()),
        "task idempotency key",
    )?;
    for task in &ledger.tasks {
        if !task.authorized
            || !task.entitlement_weight.is_finite()
            || task.entitlement_weight <= 0.0
            || !task.artifact_value.is_finite()
            || task.artifact_value < 0.0
            || !task.human_seconds.is_finite()
            || task.human_seconds < 0.0
            || task
                .role_thresholds
                .values()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            bail!("Invalid authorized task {}", task.task_id);
        }
    }
    unique(
        ledger.jobs.iter().map(|job| job.idempotency_key.as_str()),
        "job idempotency key",
    )?;
    let now = time::OffsetDateTime::now_utc();
    for account in &profile.accounts {
        if !crate::subscriptions::PROVIDERS
            .iter()
            .any(|provider| provider.id == account.provider_id)
        {
            bail!("Unknown account provider");
        }
        let entry = ledger
            .accounts
            .iter()
            .find(|entry| entry.account_id == account.id)
            .context("Missing account ledger")?;
        unique(
            entry.windows.iter().map(|window| window.window_id.as_str()),
            "ledger window ID",
        )?;
        if entry.windows.len() != account.quota_windows.len() {
            bail!("Ledger windows differ from the account quota windows");
        }
        for window in &account.quota_windows {
            let live = entry
                .windows
                .iter()
                .find(|entry| entry.window_id == window.id)
                .context("Missing quota window ledger")?;
            let snapshot = live
                .snapshot
                .value
                .as_ref()
                .context("Quota meter snapshot is unknown")?;
            let start = timestamp(
                live.window_start
                    .value
                    .as_deref()
                    .context("Quota window start is unknown")?,
            )?;
            let reset = timestamp(
                live.reset_at
                    .value
                    .as_deref()
                    .context("Quota window reset is unknown")?,
            )?;
            let observed = timestamp(&snapshot.observed_at)?;
            let capacity = window.capacity.value.context("Quota capacity is unknown")?;
            if window.unit.value.as_deref() != Some(live.unit.as_str())
                || start > observed
                || observed > now
                || reset <= now
                || start >= reset
                || snapshot.remaining < 0.0
                || snapshot.remaining > capacity
                || !snapshot.remaining.is_finite()
                || live.spent_since_snapshot < 0.0
                || !live.spent_since_snapshot.is_finite()
                || live.spent_since_snapshot > snapshot.remaining
                || snapshot.evidence.is_empty()
            {
                bail!(
                    "Quota window {} needs a current consistent native-unit snapshot",
                    window.id
                );
            }
            let held = ledger
                .jobs
                .iter()
                .flat_map(|job| &job.holds)
                .filter(|hold| hold.account_id == account.id && hold.window_id == window.id)
                .filter(|hold| {
                    hold.reflected_through.as_deref() != Some(snapshot.observed_at.as_str())
                })
                .map(|hold| hold.amount)
                .sum::<f64>();
            let external = ledger
                .external_reserves
                .iter()
                .filter(|reserve| {
                    reserve.account_id == account.id && reserve.window_id == window.id
                })
                .filter(|reserve| {
                    reserve.reflected_through.as_deref() != Some(snapshot.observed_at.as_str())
                })
                .map(|reserve| reserve.amount)
                .sum::<f64>();
            if held + external + live.spent_since_snapshot > snapshot.remaining {
                bail!(
                    "Quota holds exceed observed remaining capacity for {}",
                    window.id
                );
            }
        }
    }
    if ledger.accounts.iter().any(|entry| {
        !profile
            .accounts
            .iter()
            .any(|account| account.id == entry.account_id)
    }) {
        bail!("Ledger references an unknown account");
    }
    for job in &ledger.jobs {
        let binding = profile
            .bindings
            .iter()
            .find(|binding| binding.id == job.binding_id)
            .context("Job references an unknown model binding")?;
        let account = profile
            .accounts
            .iter()
            .find(|account| account.id == binding.account_id)
            .context("Job account is unknown")?;
        let applicable: Vec<_> = account
            .quota_windows
            .iter()
            .filter(|window| {
                window.binding_ids.is_empty() || window.binding_ids.contains(&job.binding_id)
            })
            .collect();
        unique(job.attempt_ids.iter().map(String::as_str), "attempt ID")?;
        let mut pending = job.dependencies.clone();
        let mut seen = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if id == job.id {
                bail!("Job dependency cycle");
            }
            if seen.insert(id.clone()) {
                pending.extend(
                    ledger
                        .jobs
                        .iter()
                        .find(|entry| entry.id == id)
                        .context("Unknown job dependency")?
                        .dependencies
                        .clone(),
                );
            }
        }
        for hold in &job.holds {
            if !hold.amount.is_finite()
                || hold.amount < 0.0
                || hold.account_id != binding.account_id
                || !applicable.iter().any(|window| window.id == hold.window_id)
                || hold.expected < 0.0
                || hold.expected > hold.amount
                || hold.id.is_empty()
                || hold.coefficient_id.is_empty()
            {
                bail!("Invalid job quota hold");
            }
        }
        if matches!(job.status, JobStatus::Held | JobStatus::Running)
            && applicable.iter().any(|window| {
                !job.holds
                    .iter()
                    .any(|hold| hold.window_id == window.id && hold.amount > 0.0)
            })
        {
            bail!("Active job requires holds in every applicable quota window");
        }
    }
    for reserve in &ledger.external_reserves {
        if !reserve.amount.is_finite()
            || reserve.amount < 0.0
            || reserve.evidence.is_empty()
            || !profile.accounts.iter().any(|account| {
                account.id == reserve.account_id
                    && account
                        .quota_windows
                        .iter()
                        .any(|window| window.id == reserve.window_id)
            })
        {
            bail!("Invalid external usage reserve");
        }
    }
    Ok(())
}
fn check_profile(profile: &SetupProfile) -> Result<()> {
    if profile.computer.id != computer_id() {
        bail!(
            "Computer identity differs; onboard and rebase executable and workspace paths on this computer"
        );
    }
    facts(&serde_json::to_value(profile)?)?;
    unique(profile.hosts.iter().map(|host| host.id.as_str()), "host ID")?;
    unique(
        profile.accounts.iter().map(|account| account.id.as_str()),
        "account ID",
    )?;
    unique(
        profile.bindings.iter().map(|binding| binding.id.as_str()),
        "model binding ID",
    )?;
    let host_ids: BTreeSet<_> = profile.hosts.iter().map(|host| host.id.as_str()).collect();
    let account_ids: BTreeSet<_> = profile
        .accounts
        .iter()
        .map(|account| account.id.as_str())
        .collect();
    let binding_ids: BTreeSet<_> = profile
        .bindings
        .iter()
        .map(|binding| binding.id.as_str())
        .collect();
    unique(
        profile
            .native_coefficients
            .iter()
            .map(|coefficient| coefficient.id.as_str()),
        "native coefficient ID",
    )?;
    unique(
        profile
            .capability_bundles
            .iter()
            .map(|bundle| bundle.id.as_str()),
        "capability bundle ID",
    )?;
    unique(
        profile
            .role_evidence
            .iter()
            .map(|record| record.id.as_str()),
        "role evidence ID",
    )?;
    unique(
        profile
            .project_policies
            .iter()
            .map(|policy| policy.project_id.as_str()),
        "project policy ID",
    )?;
    for policy in &profile.project_policies {
        if !policy.entitlement_weight.is_finite()
            || policy.entitlement_weight <= 0.0
            || policy.evidence.is_empty()
            || policy
                .role_thresholds
                .values()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            bail!("Invalid project policy {}", policy.project_id);
        }
    }
    for bundle in &profile.capability_bundles {
        timestamp(&bundle.observed_at)?;
        if !binding_ids.contains(bundle.binding_id.as_str())
            || bundle.permission.is_empty()
            || bundle.launch_identity.is_empty()
            || bundle.evidence.is_empty()
        {
            bail!("Invalid capability bundle {}", bundle.id);
        }
    }
    for record in &profile.role_evidence {
        timestamp(&record.observed_at)?;
        if !binding_ids.contains(record.binding_id.as_str())
            || record.dimensions.is_empty()
            || record.benchmark.is_empty()
            || record.evidence.is_empty()
            || record
                .lower_bound
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
            || record
                .required_lower_bound
                .is_some_and(|value| !value.is_finite() || !(0.0..=1.0).contains(&value))
        {
            bail!("Invalid or stale role evidence {}", record.id);
        }
    }
    for coefficient in &profile.native_coefficients {
        if !binding_ids.contains(coefficient.binding_id.as_str())
            || !coefficient.expected.is_finite()
            || !coefficient.reserve.is_finite()
            || coefficient.expected < 0.0
            || coefficient.reserve < coefficient.expected
            || coefficient.account_id.is_empty()
            || coefficient.unit.is_empty()
            || !coefficient.expected_seconds.is_finite()
            || !coefficient.reserve_seconds.is_finite()
            || coefficient.expected_seconds < 0.0
            || coefficient.reserve_seconds < coefficient.expected_seconds
            || !coefficient.uncertainty_lower.is_finite()
            || !coefficient.uncertainty_upper.is_finite()
            || coefficient.uncertainty_lower < 0.0
            || coefficient.uncertainty_upper < coefficient.uncertainty_lower
            || coefficient.sample_count == 0 && !coefficient.manual_pilot
            || coefficient.observed_from.trim().is_empty()
            || coefficient.observed_to.trim().is_empty()
            || coefficient.estimator.trim().is_empty()
            || coefficient.uncertainty.trim().is_empty()
            || coefficient.evidence.is_empty()
            || !profile.accounts.iter().any(|account| {
                account.id == coefficient.account_id
                    && account.quota_windows.iter().any(|window| {
                        window.id == coefficient.window_id
                            && window.unit.value.as_deref() == Some(coefficient.unit.as_str())
                            && (window.binding_ids.is_empty()
                                || window.binding_ids.contains(&coefficient.binding_id))
                    })
            })
        {
            bail!("Invalid native coefficient {}", coefficient.id);
        }
    }
    for host in &profile.hosts {
        environment(&host.environment)?;
        if host
            .active_binding_id
            .as_ref()
            .and_then(|fact| fact.value.as_ref())
            .is_some_and(|id| {
                !profile
                    .bindings
                    .iter()
                    .any(|binding| &binding.id == id && binding.host_id == host.id)
            })
        {
            bail!("Host {} has an invalid active model binding", host.id);
        }
        if host
            .executable
            .value
            .as_ref()
            .is_some_and(|path| !Path::new(path).is_absolute())
            || host
                .account_ids
                .iter()
                .any(|id| !account_ids.contains(id.as_str()))
        {
            bail!(
                "Host {} has invalid executable or account references",
                host.id
            );
        }
    }
    for (provider, binding) in &profile.providers {
        if !crate::subscriptions::PROVIDERS
            .iter()
            .any(|item| item.id == provider)
            || !profile
                .accounts
                .iter()
                .any(|account| account.id == binding.account_id && account.provider_id == *provider)
            || binding
                .host_ids
                .iter()
                .any(|id| !host_ids.contains(id.as_str()))
        {
            bail!("Invalid shared account/host mapping for {provider}");
        }
    }
    for binding in &profile.bindings {
        if binding
            .billing
            .as_ref()
            .and_then(|fact| fact.value.as_ref())
            .is_some_and(|billing| billing.connector.trim().is_empty())
        {
            bail!("Model billing connector must identify the authorized billing route");
        }
        if !host_ids.contains(binding.host_id.as_str())
            || !profile.accounts.iter().any(|account| {
                account.id == binding.account_id && account.provider_id == binding.provider_id
            })
            || !profile.hosts.iter().any(|host| {
                host.id == binding.host_id
                    && host.account_ids.contains(&binding.account_id)
                    && (!binding.subscription_funded(false)
                        || host
                            .kind
                            .native_provider()
                            .is_none_or(|provider| provider == binding.provider_id))
            })
        {
            bail!("Invalid model routing references for {}", binding.id);
        }
    }
    for account in &profile.accounts {
        unique(
            account
                .quota_windows
                .iter()
                .map(|window| window.id.as_str()),
            "quota window ID",
        )?;
        for window in &account.quota_windows {
            if [window.capacity.value]
                .into_iter()
                .flatten()
                .any(|value| !value.is_finite() || value < 0.0)
            {
                bail!("Invalid quota amount for {}", window.id);
            }
            if window.binding_ids.iter().any(|id| {
                !binding_ids.contains(id.as_str())
                    || !profile
                        .bindings
                        .iter()
                        .any(|binding| &binding.id == id && binding.account_id == account.id)
            }) {
                bail!("Unknown model in quota subset {}", window.id);
            }
            let mut seen = BTreeSet::new();
            let mut parent = window.parent_window.as_ref();
            while let Some(id) = parent {
                if id == &window.id || !seen.insert(id) {
                    bail!("Quota window cycle at {}", window.id);
                }
                let ancestor = account
                    .quota_windows
                    .iter()
                    .find(|item| &item.id == id)
                    .context("Unknown parent quota window")?;
                if !ancestor.binding_ids.is_empty()
                    && (window.binding_ids.is_empty()
                        || window
                            .binding_ids
                            .iter()
                            .any(|binding| !ancestor.binding_ids.contains(binding)))
                {
                    bail!("Nested quota bindings must be a subset of every parent window");
                }
                parent = ancestor.parent_window.as_ref();
            }
            if let Some(meter) = &window.meter.value {
                environment(&meter.environment)?;
                if !meter.read_only || !Path::new(&meter.executable).is_absolute() {
                    bail!("Quota meter must be a confirmed read-only absolute command");
                }
            }
        }
    }
    if profile
        .workflow
        .available_hours
        .value
        .is_some_and(|value| !value.is_finite() || value < 0.0)
        || profile.workflow.concurrency.value == Some(0)
        || profile
            .workflow
            .orchestrators
            .as_ref()
            .and_then(|fact| fact.value)
            .is_some_and(|count| !(1..=64).contains(&count))
    {
        bail!("Invalid workflow hours or concurrency");
    }
    for period in profile.workflow.work_periods.value.iter().flatten() {
        let format = time::format_description::parse_borrowed::<2>("[hour]:[minute]")?;
        let start = time::Time::parse(&period.start, &format)?;
        let end = time::Time::parse(&period.end, &format)?;
        if period.weekday > 6 || start >= end {
            bail!(
                "Work periods require weekday 0..6 and increasing HH:MM times; split overnight periods"
            );
        }
    }
    for window in profile
        .workflow
        .active_windows
        .iter()
        .filter_map(|fact| fact.value.as_ref())
        .flatten()
    {
        if timestamp(&window.start)? >= timestamp(&window.end)? {
            bail!("Active windows require increasing RFC3339 bounds");
        }
    }
    if profile
        .workflow
        .human_available_seconds
        .as_ref()
        .and_then(|fact| fact.value)
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        bail!("Invalid human attention capacity");
    }
    for path in profile
        .workflow
        .roots
        .value
        .iter()
        .flatten()
        .chain(profile.workflow.instructions.value.iter().flatten())
    {
        if !Path::new(path).is_absolute() {
            bail!("Workspace and instruction paths must be absolute");
        }
    }
    if profile
        .orchestrator_host_override
        .value
        .as_ref()
        .is_some_and(|value| !host_ids.contains(value.host_id.as_str()))
        || profile
            .orchestrator_overhead_account
            .value
            .as_ref()
            .is_some_and(|id| !account_ids.contains(id.as_str()))
    {
        bail!("Invalid orchestrator host/account override");
    }
    Ok(())
}
pub fn load_setup(table: &Table) -> Result<SetupState> {
    let paths = paths()?;
    load_setup_at(table, &paths)
}

pub fn load_setup_at(table: &Table, paths: &SetupPaths) -> Result<SetupState> {
    if !paths.profile.exists() {
        return Ok(SetupState {
            profile: None,
            ready: false,
            message: "Onboarding required: setup.v1.json is absent".into(),
            issues: vec![],
        });
    }
    let profile = match read_json::<SetupProfile>(&paths.profile).and_then(|profile| {
        check_profile(&profile)?;
        Ok(profile)
    }) {
        Ok(profile) => profile,
        Err(error) => {
            return Ok(SetupState {
                profile: None,
                ready: false,
                message: format!("Onboarding required: {error:#}"),
                issues: vec![],
            });
        }
    };
    let mut issues = Vec::new();
    if let Some(portfolio) = &table.portfolio {
        if profile.workflow.available_hours.value != Some(portfolio.available_hours_per_provider) {
            issues.push("Confirm the current available hours".into());
        }
        if profile
            .workflow
            .orchestrators
            .as_ref()
            .and_then(|fact| fact.value)
            != Some(portfolio.orchestrators)
        {
            issues.push("Confirm the current project orchestrator count".into());
        }
        for pool in portfolio.pools.iter() {
            let account = profile
                .providers
                .get(&pool.provider_id)
                .and_then(|binding| {
                    profile
                        .accounts
                        .iter()
                        .find(|account| account.id == binding.account_id)
                });
            if account.is_none_or(|account| {
                account.plan.value.as_deref() != Some(&pool.plan_id)
                    || account.authenticated.value != Some(true)
                    || account.quota_windows.is_empty()
                    || account.quota_windows.iter().any(|window| {
                        window.unit.value.is_none()
                            || window.capacity.value.is_none()
                            || window.reset_rule.value.is_none()
                    })
            }) {
                issues.push(format!(
                    "Confirm {} plan, authentication, native quota windows and resets",
                    pool.provider_name
                ));
            }
        }
        if let Some(conductor) = &portfolio.conductor
            && let Some(row) = table.rows.get(conductor.row_index)
        {
            let id = &conductor.binding_id;
            if profile
                .bindings
                .iter()
                .find(|binding| binding.id == *id)
                .is_none_or(|binding| {
                    binding.display_model != row.model
                        || binding.display_effort.status == FactStatus::Unknown
                        || binding.display_effort.value.as_deref()
                            != Some(row.effort.as_deref().unwrap_or("none"))
                        || binding.provider_id != conductor.provider_id
                        || binding.entitlement.value != Some(true)
                        || !binding.subscription_funded(false)
                        || binding.native_model.value.is_none()
                        || binding.model_args.value.is_none()
                        || binding.effort_args.value.is_none()
                        || profile
                            .providers
                            .get(&conductor.provider_id)
                            .is_none_or(|provider| {
                                provider.account_id != binding.account_id
                                    || !provider.host_ids.contains(&binding.host_id)
                            })
                })
            {
                issues.push(format!(
                    "Confirm fixed orchestrator model/effort binding {id}: {}",
                    row.display_name()
                ));
            }
            let conductor_account = profile
                .providers
                .get(&conductor.provider_id)
                .map(|provider| provider.account_id.as_str());
            if profile.orchestrator_overhead_account.status == FactStatus::Unknown
                || profile.orchestrator_overhead_account.value.as_deref() != conductor_account
            {
                issues.push(format!(
                    "Bind orchestrator overhead to the fixed {} provider account",
                    conductor.provider_id
                ));
            }
        }
        for role in portfolio
            .roles
            .iter()
            .filter(|role| role.seat != Seat::Orchestrator)
        {
            for rule in &role.rules {
                for (index, provider_id, primary) in rule
                    .row_index
                    .zip(rule.provider_id.as_deref())
                    .into_iter()
                    .map(|(index, provider)| (index, provider, true))
                    .chain(
                        rule.lower_effort
                            .iter()
                            .chain(&rule.within_provider_alternatives)
                            .chain(&rule.surplus_alternatives)
                            .map(|route| (route.row_index, route.provider_id.as_str(), false)),
                    )
                {
                    let row = &table.rows[index];
                    let id = binding_id(row);
                    if profile
                        .bindings
                        .iter()
                        .find(|binding| binding.id == id)
                        .is_none_or(|binding| {
                            if binding.display_model != row.model
                                || binding.display_effort.status == FactStatus::Unknown
                                || binding.display_effort.value.as_deref()
                                    != Some(row.effort.as_deref().unwrap_or("none"))
                                || binding.provider_id != provider_id
                                || profile.providers.get(provider_id).is_none_or(|provider| {
                                    provider.account_id != binding.account_id
                                        || !provider.host_ids.contains(&binding.host_id)
                                })
                            {
                                return true;
                            }
                            if !primary
                                && binding.entitlement.status != FactStatus::Unknown
                                && binding.entitlement.value == Some(false)
                            {
                                return false;
                            }
                            binding.entitlement.value != Some(true)
                                || !binding.subscription_funded(false)
                                || binding.native_model.value.is_none()
                                || binding.model_args.value.is_none()
                                || binding.effort_args.value.is_none()
                                || !profile.hosts.iter().any(|host| {
                                    host.id == binding.host_id
                                        && host.installed.value == Some(true)
                                        && (!host.kind.is_multi_provider()
                                            || (binding.subscription_funded(true)
                                                && host
                                                    .active_binding_id
                                                    .as_ref()
                                                    .and_then(|fact| fact.value.as_ref())
                                                    .is_some()))
                                        && host.lifecycle.value.is_some()
                                        && host.background.value.is_some()
                                        && host
                                            .executable
                                            .value
                                            .as_ref()
                                            .is_some_and(|path| Path::new(path).is_file())
                                })
                        })
                    {
                        issues.push(format!(
                            "Confirm native model/effort binding {id}: {}",
                            row.display_name()
                        ));
                    }
                }
            }
        }
    }
    if profile.workflow.roots.value.is_none()
        || profile.workflow.timezone.value.is_none()
        || profile.workflow.work_periods.value.is_none()
        || profile.workflow.network.value.is_none()
        || profile.workflow.instructions.value.is_none()
        || profile.workflow.exclusive_resources.value.is_none()
        || profile.workflow.background.value.is_none()
        || profile.workflow.tools.value.is_none()
        || profile.workflow.exclusions.value.is_none()
        || profile.workflow.approval.value.is_none()
        || profile.workflow.checks.value.is_none()
        || profile.workflow.concurrency.value.is_none()
        || profile
            .workflow
            .active_windows
            .as_ref()
            .and_then(|fact| fact.value.as_ref())
            .is_none()
        || profile
            .workflow
            .human_available_seconds
            .as_ref()
            .and_then(|fact| fact.value)
            .is_none()
        || profile.orchestrator_overhead_account.value.is_none()
    {
        issues.push(
            "Confirm workspace, checks, approval, concurrency and orchestrator overhead account"
                .into(),
        );
    }
    if let Err(error) =
        read_json::<Ledger>(&paths.ledger).and_then(|ledger| check_ledger(&ledger, &profile))
    {
        issues.push(format!("Initialize/reconcile ledger: {error:#}"));
    }
    issues.sort();
    issues.dedup();
    Ok(SetupState {
        ready: issues.is_empty(),
        message: if issues.is_empty() {
            "Accepted setup is ready for the current plan".into()
        } else {
            "Onboarding must resolve the listed routing facts before delegation".into()
        },
        profile: Some(profile),
        issues,
    })
}
fn object(properties: Value) -> Value {
    let required: Vec<_> = properties.as_object().unwrap().keys().cloned().collect();
    json!({"type":"object","additionalProperties":false,"required":required,"properties":properties})
}
fn optional_property(mut schema: Value, name: &str, value: Value) -> Value {
    schema["properties"][name] = json!({"anyOf":[value,{"type":"null"}]});
    schema
}
fn reference(name: &str) -> Value {
    json!({"$ref":format!("#/$defs/{name}")})
}
fn array(value: Value) -> Value {
    json!({"type":"array","items":value})
}
fn fact(value: Value) -> Value {
    let name = if let Some(name) = value
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|value| value.strip_prefix("#/$defs/"))
    {
        format!("Fact{name}")
    } else {
        match value.get("type").and_then(Value::as_str) {
            Some("boolean") => "FactBool",
            Some("number") => "FactNonnegative",
            Some("integer") => "FactPositiveInt",
            Some("array") if value["items"]["$ref"] == "#/$defs/WorkPeriod" => "FactWorkPeriods",
            Some("array") => "FactStringArray",
            _ => "FactString",
        }
        .to_owned()
    };
    reference(&name)
}
fn fact_definition(value: Value) -> Value {
    let mut schema = object(
        json!({"status":{"enum":["verified","confirmed","unknown"]},"value":{"anyOf":[value,{"type":"null"}]},"evidence":array(json!({"type":"string"}))}),
    );
    schema["allOf"] = json!([{"if":{"properties":{"status":{"const":"unknown"}}},"then":{"properties":{"value":{"type":"null"}}},"else":{"properties":{"value":{"not":{"type":"null"}},"evidence":{"minItems":1}}}}]);
    schema
}
fn definitions() -> Value {
    let string = json!({"type":"string","minLength":1});
    let id = json!({"type":"string","pattern":"^[a-z][a-z0-9-]{0,95}$"});
    let strings = array(string.clone());
    let boolean = json!({"type":"boolean"});
    let number = json!({"type":"number","minimum":0});
    let environment = json!({"type":"object","additionalProperties":{"type":"string"},"propertyNames":{"enum":["CODEX_HOME","CLAUDE_CONFIG_DIR","GROK_HOME","GEMINI_CLI_HOME"]}});
    json!({
        "FactString":fact_definition(string.clone()),
        "FactBool":fact_definition(boolean.clone()),
        "FactNonnegative":fact_definition(number.clone()),
        "FactStringArray":fact_definition(strings.clone()),
        "FactPositiveInt":fact_definition(json!({"type":"integer","minimum":1,"maximum":4294967295_u64})),
        "FactMeterCommand":fact_definition(reference("MeterCommand")),
        "FactWorkPeriods":fact_definition(array(reference("WorkPeriod"))),
        "FactActiveWindows":fact_definition(array(reference("ActiveWindow"))),
        "FactHostOverride":fact_definition(reference("HostOverride")),
        "FactMeterSnapshot":fact_definition(reference("MeterSnapshot")),
        "FactBillingBinding":fact_definition(reference("BillingBinding")),
        "NativeCoefficient":object(json!({"id":id,"binding_id":string,"seat":{"enum":["Implementer","Debugger","Reviewer","Orchestrator","Sanity","Comprehension","NetResearch"]},"workflow":{"enum":["Focused","Standard","Complex","Extensive"]},"window_id":string,"account_id":string,"unit":string,"expected":number,"reserve":number,"expected_seconds":number,"reserve_seconds":number,"uncertainty_lower":number,"uncertainty_upper":number,"manual_pilot":boolean,"sample_count":{"type":"integer","minimum":0},"observed_from":string,"observed_to":string,"estimator":string,"uncertainty":string,"evidence":strings})),
        "RiskVector":object(json!({"consequence":{"type":"integer","minimum":0,"maximum":3},"uncertainty":{"type":"integer","minimum":0,"maximum":3},"coupling":{"type":"integer","minimum":0,"maximum":3},"reversibility":{"type":"integer","minimum":0,"maximum":3},"evidence_need":{"type":"integer","minimum":0,"maximum":3},"tool_risk":{"type":"integer","minimum":0,"maximum":3},"correlation":{"type":"integer","minimum":0,"maximum":3},"deadline":{"type":"integer","minimum":0,"maximum":3}})),
        "TaskRequest":object(json!({"task_id":id,"project_id":id,"generation_id":id,"workspace_id":id,"idempotency_key":string,"brief_path":string,"timeout_seconds":{"type":"integer","minimum":1,"maximum":86400},"authorized":boolean,"ready":boolean,"risk":reference("RiskVector"),"evidence_requirements":strings,"dependencies":strings,"priority":{"type":"integer","minimum":0},"entitlement_weight":number,"deadline":{"anyOf":[string.clone(),{"type":"null"}]},"ready_at":{"anyOf":[string.clone(),{"type":"null"}]},"artifact_value":number,"required_resources":strings,"human_seconds":number,"role_thresholds":{"type":"object","additionalProperties":number.clone()}})),
        "CapabilityBundle":object(json!({"id":id,"binding_id":string,"tools":strings,"permission":string,"network_scope":strings,"source_scope":strings,"roots":strings,"exclusions":strings,"isolated":boolean,"observed_at":string,"freshness_seconds":{"type":"integer","minimum":1},"evidence":strings,"launch_identity":string})),
        "RoleEvidence":object(json!({"id":id,"binding_id":string,"seat":{"enum":["Implementer","Debugger","Reviewer","Orchestrator","Sanity","Comprehension","NetResearch"]},"dimensions":strings,"confidence":{"enum":["provisional","calibrated"]},"benchmark":string,"transfer_assumption":{"anyOf":[string.clone(),{"type":"null"}]},"lower_bound":{"anyOf":[number.clone(),{"type":"null"}]},"required_lower_bound":{"anyOf":[number.clone(),{"type":"null"}]},"observed_at":string,"freshness_seconds":{"type":"integer","minimum":1},"evidence":strings})),
        "ProjectPolicy":object(json!({"project_id":id,"priority":{"type":"integer","minimum":0},"entitlement_weight":number,"minimum_workflow":{"enum":["Focused","Standard","Complex","Extensive"]},"role_thresholds":{"type":"object","additionalProperties":number.clone()},"evidence":strings})),
        "BillingBinding":object(json!({"mode":{"enum":["subscription","api"]},"connector":string})),
        "ComputerProfile":object(json!({"id":id,"hostname":fact(string.clone()),"platform":fact(string.clone())})),
        "HostProfile":optional_property(optional_property(object(json!({"id":id,"kind":{"enum":["codex","claude","muse","gemini","antigravity","grok","opencode","aider","goose","cline","openhands","buzz","pi","droid"]},"installed":fact(boolean.clone()),"executable":fact(string.clone()),"argv":strings,"environment":environment,"account_ids":strings,"background":fact(boolean.clone()),"lifecycle":fact(string.clone())})),"active_binding_id",fact(string.clone())),"isolation",fact(reference("IsolationBinding"))),
        "MeterCommand":object(json!({"executable":string,"argv":strings,"environment":environment,"read_only":{"const":true}})),
        "QuotaWindow":object(json!({"id":id,"unit":fact(string.clone()),"capacity":fact(number.clone()),"reset_rule":fact(string.clone()),"parent_window":{"anyOf":[string.clone(),{"type":"null"}]},"binding_ids":strings,"meter":fact(reference("MeterCommand"))})),
        "AccountProfile":object(json!({"id":id,"provider_id":string,"plan":fact(string.clone()),"authenticated":fact(boolean.clone()),"quota_windows":array(reference("QuotaWindow"))})),
        "ProviderBinding":object(json!({"account_id":string,"host_ids":strings})),
        "IsolationBinding":object(json!({"generation_directory":string,"account_ids":strings,"executable_identity":string,"executable_version":string})),
        "ModelBinding":optional_property(object(json!({"id":id,"provider_id":string,"host_id":string,"account_id":string,"display_model":string,"display_effort":fact(string.clone()),"native_model":fact(string.clone()),"model_args":fact(strings.clone()),"effort_args":fact(strings.clone()),"entitlement":fact(boolean.clone())})),"billing",fact(reference("BillingBinding"))),
        "ActiveWindow":object(json!({"start":string,"end":string})),
        "WorkPeriod":object(json!({"weekday":{"type":"integer","minimum":0,"maximum":6},"start":string,"end":string})),"HostOverride":object(json!({"host_id":string,"recommended_provider":string})),"WorkflowProfile":optional_property(optional_property(optional_property(object(json!({"timezone":fact(string.clone()),"work_periods":fact(array(reference("WorkPeriod"))),"network":fact(string.clone()),"instructions":fact(strings.clone()),"exclusive_resources":fact(strings.clone()),"roots":fact(strings.clone()),"exclusions":fact(strings.clone()),"tools":fact(strings.clone()),"background":fact(boolean.clone()),"concurrency":fact(json!({"type":"integer","minimum":1,"maximum":4294967295_u64})),"approval":fact(string.clone()),"checks":fact(strings.clone()),"available_hours":fact(number.clone())})),"orchestrators",fact_definition(json!({"type":"integer","minimum":1,"maximum":64}))),"active_windows",fact_definition(array(reference("ActiveWindow")))),"human_available_seconds",fact_definition(number.clone())),
        "MeterSnapshot":object(json!({"observed_at":string,"remaining":{"type":"number","minimum":0},"evidence":array(string.clone())})),
        "WindowLedger":object(json!({"window_id":string,"unit":string,"window_start":fact(string.clone()),"reset_at":fact(string.clone()),"snapshot":fact(reference("MeterSnapshot")),"spent_since_snapshot":{"type":"number","minimum":0}})),
        "AccountLedger":object(json!({"account_id":string,"windows":array(reference("WindowLedger"))})),
        "Hold":object(json!({"id":string,"account_id":string,"window_id":string,"amount":number,"expected":number,"coefficient_id":string,"reflected_through":{"anyOf":[string.clone(),{"type":"null"}]},"window_start":{"anyOf":[string.clone(),{"type":"null"}]},"reset_at":{"anyOf":[string.clone(),{"type":"null"}]}})),
        "UsageSettlement":object(json!({"settlement_id":string,"hold_id":string,"account_id":string,"window_id":string,"unit":string,"amount":number,"attribution":string,"observed_at":string,"provider_event_id":{"anyOf":[string.clone(),{"type":"null"}]},"window_start":{"anyOf":[string.clone(),{"type":"null"}]},"reset_at":{"anyOf":[string.clone(),{"type":"null"}]}})),
        "AttemptOutcome":object(json!({"category":string,"artifact_accepted":boolean,"rejection_reason":{"anyOf":[string.clone(),{"type":"null"}]},"rework_cause":{"anyOf":[string.clone(),{"type":"null"}]},"human_escalation":boolean})),
        "ImmutableArtifact":object(json!({"id":id,"path":string,"digest":string,"producer_attempt_id":string,"parent_artifact_id":{"anyOf":[string.clone(),{"type":"null"}]},"accepted_at":{"anyOf":[string.clone(),{"type":"null"}]}})),
        "HorizonPointer":object(json!({"id":id,"policy_version":string,"profile_identity":string,"committed_revision":{"type":"integer","minimum":1},"artifact_path":string,"generation_id":string})),
        "ExternalReserve":object(json!({"id":id,"account_id":string,"window_id":string,"amount":number,"reflected_through":{"anyOf":[string.clone(),{"type":"null"}]},"evidence":strings})),
        "JobLedger":object(json!({"id":id,"idempotency_key":string,"binding_id":string,"route_id":string,"status":{"enum":["queued","ready","blocked","cancelled","held","running","completed","released","deferred"]},"dependencies":strings,"attempt_ids":strings,"worker_handle":fact(string.clone()),"artifacts":strings,"artifact_records":array(reference("ImmutableArtifact")),"recovery":fact(string.clone()),"holds":array(reference("Hold")),"project_id":string,"parent_task_id":string,"workflow":{"anyOf":[{"enum":["Focused","Standard","Complex","Extensive"]},{"type":"null"}]},"seat":{"anyOf":[{"enum":["Implementer","Debugger","Reviewer","Orchestrator","Sanity","Comprehension","NetResearch"]},{"type":"null"}]},"risk":{"anyOf":[reference("RiskVector"),{"type":"null"}]},"evidence_requirements":strings,"conditional":boolean,"actual_settled":array(reference("UsageSettlement")),"outcome":{"anyOf":[reference("AttemptOutcome"),{"type":"null"}]},"scheduled_start":{"anyOf":[string.clone(),{"type":"null"}]},"scheduled_end":{"anyOf":[string.clone(),{"type":"null"}]},"remaining_seconds":{"anyOf":[number.clone(),{"type":"null"}]},"settlement_keys":strings,"settlement_digests":{"type":"object","additionalProperties":string.clone()},"started_at":{"anyOf":[string.clone(),{"type":"null"}]},"ended_at":{"anyOf":[string.clone(),{"type":"null"}]},"input_context_bucket":{"anyOf":[string.clone(),{"type":"null"}]},"tool_calls":{"type":"object","additionalProperties":{"type":"integer","minimum":0}}}))
    })
}
pub fn setup_schema() -> String {
    let mut schema = object(
        json!({"version":{"const":1},"computer":reference("ComputerProfile"),"hosts":array(reference("HostProfile")),"accounts":array(reference("AccountProfile")),"providers":{"type":"object","additionalProperties":reference("ProviderBinding")},"bindings":array(reference("ModelBinding")),"workflow":reference("WorkflowProfile"),"orchestrator_host_override":fact(reference("HostOverride")),"orchestrator_overhead_account":fact(json!({"type":"string"})),"native_coefficients":array(reference("NativeCoefficient")),"capability_bundles":array(reference("CapabilityBundle")),"role_evidence":array(reference("RoleEvidence")),"project_policies":array(reference("ProjectPolicy"))}),
    );
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema["$defs"] = reachable_definitions(&schema);
    serde_json::to_string(&schema).unwrap()
}
pub fn ledger_schema() -> String {
    let mut schema = object(
        json!({"version":{"const":1},"computer_id":{"type":"string"},"updated_at":{"type":"string"},"accounts":array(reference("AccountLedger")),"jobs":array(reference("JobLedger")),"revision":{"type":"integer","minimum":0},"external_reserves":array(reference("ExternalReserve")),"current_horizon":{"anyOf":[reference("HorizonPointer"),{"type":"null"}]},"horizons":array(reference("HorizonPointer")),"tasks":array(reference("TaskRequest")),"meter_observations":{"type":"array","items":{"type":"object"}}}),
    );
    schema["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    schema["$defs"] = reachable_definitions(&schema);
    serde_json::to_string(&schema).unwrap()
}
pub fn onboarding_example() -> Result<String> {
    let unknown = json!({"status":"unknown","value":null,"evidence":[]});
    Ok(serde_json::to_string_pretty(
        &json!({"version":1,"computer":{"id":computer_id(),"hostname":unknown,"platform":unknown},"hosts":[],"accounts":[],"providers":{},"bindings":[],"workflow":{"timezone":unknown,"work_periods":unknown,"network":unknown,"instructions":unknown,"exclusive_resources":unknown,"roots":unknown,"exclusions":unknown,"tools":unknown,"background":unknown,"concurrency":unknown,"orchestrators":unknown,"approval":unknown,"checks":unknown,"available_hours":unknown,"active_windows":unknown,"human_available_seconds":unknown},"orchestrator_host_override":unknown,"orchestrator_overhead_account":unknown,"native_coefficients":[],"capability_bundles":[],"role_evidence":[],"project_policies":[]}),
    )?)
}

fn reachable_definitions(schema: &Value) -> Value {
    fn references(value: &Value, names: &mut BTreeSet<String>) {
        match value {
            Value::Object(object) => {
                if let Some(name) = object
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|value| value.strip_prefix("#/$defs/"))
                {
                    names.insert(name.to_owned());
                }
                for child in object.values() {
                    references(child, names);
                }
            }
            Value::Array(array) => {
                for child in array {
                    references(child, names);
                }
            }
            _ => {}
        }
    }
    let all = definitions();
    let mut names = BTreeSet::new();
    references(schema, &mut names);
    loop {
        let count = names.len();
        for name in names.clone() {
            references(&all[&name], &mut names);
        }
        if names.len() == count {
            break;
        }
    }
    Value::Object(
        names
            .into_iter()
            .map(|name| {
                let value = all[&name].clone();
                (name, value)
            })
            .collect(),
    )
}
