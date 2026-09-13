use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub source_id: String,
    pub url: String,
    pub revision: String,
    pub fetched_at: String,
    pub observed_at: Option<String>,
    pub published_at: Option<String>,
    pub license: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkSeries {
    pub family: String,
    pub variant: String,
    pub version: String,
    pub version_order: Vec<u64>,
    pub task_count: Option<usize>,
    pub comparable_series_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSubject {
    pub provider: String,
    pub model: String,
    pub raw_model_id: String,
    pub effort: Option<String>,
}

impl ModelSubject {
    pub fn normalized_model(&self) -> String {
        let model = normalize_identity(&self.model);
        if canonical_provider(&self.provider) == "anthropic" {
            model.strip_prefix("claude-").unwrap_or(&model).to_owned()
        } else {
            model
        }
    }

    pub fn normalized_effort(&self) -> Option<String> {
        self.effort.as_deref().map(normalize_identity)
    }
}

impl ScoreObservation {
    pub fn normalized_value(&self) -> Option<f64> {
        let (minimum, maximum) = self.scale_min.zip(self.scale_max)?;
        (self.value.is_finite() && minimum.is_finite() && maximum > minimum).then(|| {
            let value = (self.value - minimum) / (maximum - minimum);
            if self.higher_is_better {
                value
            } else {
                1.0 - value
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionIdentity {
    pub harness: String,
    pub harness_version: Option<String>,
    pub protocol: String,
    pub grader: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreObservation {
    pub metric: String,
    pub value: f64,
    pub scale_min: Option<f64>,
    pub scale_max: Option<f64>,
    pub higher_is_better: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceObservation {
    pub seconds: Option<f64>,
    pub pooled_seconds: Option<f64>,
    pub usd: Option<f64>,
    pub input_tokens: Option<f64>,
    pub output_tokens: Option<f64>,
    #[serde(default)]
    pub time_basis: String,
    #[serde(default)]
    pub cost_basis: String,
    #[serde(default)]
    pub canonical_resources: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BenchmarkObservation {
    pub observation_id: String,
    pub source: SourceRef,
    pub series: BenchmarkSeries,
    pub subject: ModelSubject,
    pub execution: ExecutionIdentity,
    pub score: Option<ScoreObservation>,
    pub resources: Option<ResourceObservation>,
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSnapshot {
    pub source: SourceRef,
    pub payload: Option<Value>,
    pub observations: Vec<BenchmarkObservation>,
    pub fetch_error: Option<String>,
    pub last_good_revision: Option<String>,
    pub last_good_fetched_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferKind {
    ExactHarness,
    ModelLevel,
    CrossHarnessProxy,
}

#[derive(Debug, Clone, Copy)]
pub struct SelectedEvidence<'a> {
    pub observation: &'a BenchmarkObservation,
    pub transfer: TransferKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceProjection {
    pub observation_id: String,
    pub family: String,
    pub score: ScoreObservation,
    pub resources: Option<ResourceObservation>,
    pub transfer: TransferKind,
    pub source: SourceRef,
    pub series: BenchmarkSeries,
    pub execution: ExecutionIdentity,
    #[serde(default)]
    pub selection_reason: String,
    pub resource_gap: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatus {
    pub source: SourceRef,
    pub fetch_error: Option<String>,
    pub last_good_revision: Option<String>,
    pub last_good_fetched_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCatalog {
    pub sources: Vec<SourceStatus>,
    pub observations: Vec<BenchmarkObservation>,
}

impl SelectedEvidence<'_> {
    pub fn projection(&self) -> EvidenceProjection {
        let observation = self.observation;
        EvidenceProjection {
            observation_id: observation.observation_id.clone(),
            family: observation.series.family.clone(),
            score: observation
                .score
                .clone()
                .expect("selected observation score"),
            resources: observation.resources.clone(),
            transfer: self.transfer,
            source: observation.source.clone(),
            series: observation.series.clone(),
            execution: observation.execution.clone(),
            selection_reason: match self.transfer {
                TransferKind::ExactHarness => "Matching-harness evidence is selected before cross-harness proxies; freshness is compared only within this tier.".to_owned(),
                TransferKind::ModelLevel => "No matching-harness evidence is published. Exact-model-and-effort model-level evidence is selected before cross-harness proxies; freshness is compared only within this tier.".to_owned(),
                TransferKind::CrossHarnessProxy => {
                    "No exact-harness or eligible model-level evidence is published; using the newest enabled cross-harness proxy."
                        .to_owned()
                }
            },
            resource_gap: observation
                .resources
                .as_ref()
                .is_none_or(|resources| {
                    resources
                        .seconds
                        .is_none_or(|value| !value.is_finite() || value < 0.0)
                        || resources
                            .usd
                            .is_none_or(|value| !value.is_finite() || value < 0.0)
                })
                .then(|| {
                    "The selected score observation publishes no complete matching time-and-cost resource bundle."
                        .to_owned()
                }),
        }
    }
}

pub fn select_latest<'a>(
    observations: &'a [BenchmarkObservation],
    subject: &ModelSubject,
    execution: &ExecutionIdentity,
    family: &str,
    comparable_series_id: &str,
    allow_cross_harness_proxy: bool,
) -> Option<SelectedEvidence<'a>> {
    let newest = observations
        .iter()
        .filter(|observation| {
            observation.series.family == family
                && observation.series.comparable_series_id == comparable_series_id
                && same_subject(&observation.subject, subject)
                && is_primary_score(observation)
        })
        .max_by(compare_observation)?;
    let exact = observations
        .iter()
        .filter(|observation| {
            observation.series.family == family
                && observation.series.comparable_series_id == comparable_series_id
                && same_subject(&observation.subject, subject)
                && same_execution(&observation.execution, execution)
                && is_primary_score(observation)
        })
        .max_by(compare_observation);
    exact
        .map(|observation| SelectedEvidence {
            observation,
            transfer: TransferKind::ExactHarness,
        })
        .or_else(|| {
            observations
                .iter()
                .filter(|observation| {
                    observation.series.family == family
                        && observation.series.comparable_series_id == comparable_series_id
                        && same_subject(&observation.subject, subject)
                        && is_primary_score(observation)
                        && is_model_level_family(&observation.series.family)
                        && normalize_identity(&observation.execution.harness)
                            .contains("model-evaluation")
                })
                .max_by(compare_observation)
                .map(|observation| SelectedEvidence {
                    observation,
                    transfer: TransferKind::ModelLevel,
                })
        })
        .or_else(|| {
            allow_cross_harness_proxy.then_some(SelectedEvidence {
                observation: newest,
                transfer: TransferKind::CrossHarnessProxy,
            })
        })
}

pub(crate) fn is_model_level_family(family: &str) -> bool {
    matches!(
        family,
        "gpqa" | "hle" | "lcr" | "omniscience" | "intelligence-index" | "gdp-pdf"
    )
}

fn is_primary_score(observation: &BenchmarkObservation) -> bool {
    let Some(score) = &observation.score else {
        return false;
    };
    match (
        observation.series.family.as_str(),
        observation.series.comparable_series_id.as_str(),
    ) {
        ("swe", "deep-swe") => score.metric == "pass@1",
        ("terminal", "terminal-bench" | "terminal-science") => {
            matches!(
                score.metric.as_str(),
                "accuracy" | "terminalbenchV40" | "terminalbenchV21"
            )
        }
        ("qna", "swe-atlas-qna") => score.metric == "accuracy",
        ("gpqa", "gpqa-diamond-aa") => score.metric == "gpqa",
        ("hle", "hle-text-only-aa") => score.metric == "hle",
        ("lcr", "aa-lcr") => score.metric == "lcr",
        ("omniscience", "aa-omniscience-full") => score.metric == "omniscience.accuracy",
        _ => true,
    }
}

fn compare_observation(left: &&BenchmarkObservation, right: &&BenchmarkObservation) -> Ordering {
    compare_observations(left, right)
}

pub fn compare_observations(left: &BenchmarkObservation, right: &BenchmarkObservation) -> Ordering {
    version_key(&left.series.version_order)
        .cmp(version_key(&right.series.version_order))
        .then_with(|| {
            parsed_instant(left.source.observed_at.as_deref())
                .cmp(&parsed_instant(right.source.observed_at.as_deref()))
        })
        .then_with(|| {
            parsed_instant(left.source.published_at.as_deref())
                .cmp(&parsed_instant(right.source.published_at.as_deref()))
        })
        .then_with(|| left.observation_id.cmp(&right.observation_id))
}

fn version_key(version: &[u64]) -> &[u64] {
    let length = version
        .iter()
        .rposition(|component| *component != 0)
        .map_or(0, |index| index + 1);
    &version[..length]
}

fn parsed_instant(value: Option<&str>) -> Option<i128> {
    value
        .and_then(|value| OffsetDateTime::parse(value, &Rfc3339).ok())
        .map(OffsetDateTime::unix_timestamp_nanos)
}

pub fn same_subject(left: &ModelSubject, right: &ModelSubject) -> bool {
    same_subject_ignoring_effort(left, right)
        && left.normalized_effort() == right.normalized_effort()
}

pub fn same_subject_ignoring_effort(left: &ModelSubject, right: &ModelSubject) -> bool {
    canonical_provider(&left.provider) == canonical_provider(&right.provider)
        && ((!left.raw_model_id.is_empty()
            && !right.raw_model_id.is_empty()
            && left.raw_model_id == right.raw_model_id)
            || left.normalized_model() == right.normalized_model())
}

pub(crate) fn subject_key(subject: &ModelSubject) -> (String, String, Option<String>) {
    (
        canonical_provider(&subject.provider),
        subject.normalized_model(),
        subject.normalized_effort(),
    )
}

pub(crate) fn model_key(subject: &ModelSubject) -> (String, String) {
    (
        canonical_provider(&subject.provider),
        subject.normalized_model(),
    )
}

pub(crate) fn canonical_provider(value: &str) -> String {
    match normalize_identity(value).as_str() {
        "meta" | "meta-ai" => "muse".to_owned(),
        value => value.to_owned(),
    }
}

fn same_execution(left: &ExecutionIdentity, right: &ExecutionIdentity) -> bool {
    normalize_identity(&left.harness) == normalize_identity(&right.harness)
        && right
            .harness_version
            .as_ref()
            .is_none_or(|version| left.harness_version.as_ref() == Some(version))
        && (right.protocol.is_empty() || left.protocol == right.protocol)
        && (right.grader.is_empty() || left.grader == right.grader)
}

fn normalize_identity(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
