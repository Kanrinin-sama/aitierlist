pub mod deepswe;
pub mod epoch;
pub mod fetch;
pub mod hle;
pub mod scale;
pub mod surge;
pub mod terminal;

use crate::evidence::{EvidenceCatalog, SourceRef, SourceSnapshot, SourceStatus};
use crate::types::{Benchmark, Row, TaskMetric};
use reqwest::Client;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::{OnceLock, RwLock};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

static SNAPSHOTS: OnceLock<RwLock<Vec<SourceSnapshot>>> = OnceLock::new();
static LAST_REFRESH_ATTEMPT: OnceLock<RwLock<Option<String>>> = OnceLock::new();

pub async fn refresh(client: &Client, cached: &Value) -> Vec<SourceSnapshot> {
    let previous: Vec<SourceSnapshot> = serde_json::from_value(cached.clone()).unwrap_or_default();
    fetch::refresh(client, &previous).await
}
pub fn is_stale(hours: f64) -> bool {
    LAST_REFRESH_ATTEMPT
        .get_or_init(|| RwLock::new(None))
        .read()
        .expect("upstream refresh stamp lock")
        .as_deref()
        .and_then(|stamp| OffsetDateTime::parse(stamp, &Rfc3339).ok())
        .is_none_or(|attempted| {
            (OffsetDateTime::now_utc() - attempted).as_seconds_f64() >= hours * 3600.0
        })
}

pub fn load(payloads: &Value) {
    let mut snapshots: Vec<SourceSnapshot> =
        serde_json::from_value(payloads["upstreamSnapshots"].clone()).unwrap_or_default();
    if let Some(snapshot) = aa_snapshot(payloads) {
        snapshots.push(snapshot);
    }
    snapshots.push(aa_agent_snapshot(payloads));
    *SNAPSHOTS
        .get_or_init(|| RwLock::new(Vec::new()))
        .write()
        .expect("upstream snapshot lock") = snapshots;
    *LAST_REFRESH_ATTEMPT
        .get_or_init(|| RwLock::new(None))
        .write()
        .expect("upstream refresh stamp lock") = payloads["upstreamRefreshAttemptAt"]
        .as_str()
        .map(str::to_owned);
}

fn aa_agent_snapshot(payloads: &Value) -> SourceSnapshot {
    let agents = &payloads["agents"];
    let revision = payloads["http"]["agents"]["etag"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            serde_json::to_vec(agents)
                .map(|bytes| hex::encode(Sha256::digest(bytes)))
                .unwrap_or_default()
        });
    SourceSnapshot {
        source: SourceRef {
            source_id: "artificial-analysis-coding-agents".to_owned(),
            url: "https://artificialanalysis.ai/agents/coding-agents".to_owned(),
            revision,
            fetched_at: payloads["fetchedAt"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            observed_at: None,
            published_at: None,
            license: None,
        },
        payload: None,
        observations: Vec::new(),
        fetch_error: payloads["aaRefreshError"].as_str().map(str::to_owned),
        last_good_revision: None,
        last_good_fetched_at: None,
    }
}

fn aa_snapshot(payloads: &Value) -> Option<SourceSnapshot> {
    let models = payloads["evaluation"]["models"].as_array()?;
    let fetched_at = payloads["fetchedAt"].as_str().unwrap_or_default();
    let metadata = &payloads["http"]["evaluation"];
    let revision = metadata["blobEtag"]
        .as_str()
        .or_else(|| metadata["etag"].as_str())
        .unwrap_or("unknown");
    let source = SourceRef {
        source_id: "artificial-analysis-model-manifest".to_owned(),
        url: "https://artificialanalysis.ai/evaluations/livecodebench".to_owned(),
        revision: revision.to_owned(),
        fetched_at: fetched_at.to_owned(),
        observed_at: None,
        published_at: None,
        license: None,
    };
    let catalog_revision = payloads["http"]["catalog"]["blobEtag"]
        .as_str()
        .or_else(|| payloads["http"]["catalog"]["etag"].as_str())
        .unwrap_or("unknown");
    let fields = [
        "analystAgent",
        "analystAgentPassAt1",
        "analystAgentPassAtK",
        "apexAgents",
        "automationBenchPartialScore",
        "briefcaseElo",
        "critpt",
        "cweBench",
        "enterpriseOpsGym",
        "enterpriseOpsGymAvgConversationTurns",
        "gdpPdfAllPass",
        "gdpval",
        "globalMmluLiteScore",
        "gpqa",
        "harveyLab",
        "hle",
        "ifbench",
        "intelligenceIndex",
        "itbenchAvgTurnsPerTask",
        "itbenchSre",
        "lcr",
        "livecodebench",
        "math500",
        "mlcrAccuracy",
        "mlcrCompleteness",
        "mlcrConciseness",
        "mlcrOverall",
        "mmluPro",
        "mmmuPro",
        "omniscience",
        "scicode",
        "tau2",
        "tauBanking",
        "terminalbenchHard",
        "terminalbenchV21",
        "terminalbenchV40",
    ];
    let breakdowns = [
        "automationBenchBreakdown",
        "briefcaseBreakdown",
        "gdpPdfBreakdown",
        "gdpvalBreakdown",
        "harveyLabBreakdown",
        "omniscienceBreakdown",
    ];
    let mut observations = Vec::new();
    for model in models {
        let name = model["name"].as_str().unwrap_or_default();
        let effort = crate::aa::effort_of(&crate::aa::harness_key(name));
        let display = crate::aa::display_name(name);
        let model_name = effort
            .as_deref()
            .and_then(|effort| display.strip_suffix(&format!(" ({effort})")))
            .unwrap_or(&display)
            .to_owned();
        let subject = crate::evidence::ModelSubject {
            provider: model["creator"]["slug"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            model: model_name,
            raw_model_id: model["slug"].as_str().unwrap_or_default().to_owned(),
            effort,
        };
        for field in fields {
            if let Some(value) = model[field].as_f64().filter(|value| value.is_finite()) {
                observations.push(aa_observation(
                    &source,
                    &subject,
                    field,
                    value,
                    BTreeMap::from([(
                        "catalogRevision".to_owned(),
                        Value::String(catalog_revision.to_owned()),
                    )]),
                ));
            }
        }
        for field in breakdowns {
            let Some(values) = model[field].as_object() else {
                continue;
            };
            if field == "omniscienceBreakdown" {
                if let Some(accuracy) = values.get("accuracy").and_then(Value::as_f64) {
                    let mut extra: BTreeMap<_, _> = values
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect();
                    extra.insert(
                        "catalogRevision".to_owned(),
                        Value::String(catalog_revision.to_owned()),
                    );
                    observations.push(aa_observation(
                        &source,
                        &subject,
                        "omniscience.accuracy",
                        accuracy,
                        extra,
                    ));
                }
                continue;
            }
            let mut extra: BTreeMap<_, _> = values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            extra.insert(
                "catalogRevision".to_owned(),
                Value::String(catalog_revision.to_owned()),
            );
            let mut observation = aa_observation(&source, &subject, field, 0.0, extra);
            observation.score = None;
            observations.push(observation);
        }
    }
    Some(SourceSnapshot {
        source,
        payload: None,
        observations,
        fetch_error: payloads["aaRefreshError"].as_str().map(str::to_owned),
        last_good_revision: None,
        last_good_fetched_at: None,
    })
}

fn aa_observation(
    source: &SourceRef,
    subject: &crate::evidence::ModelSubject,
    field: &str,
    value: f64,
    extra: std::collections::BTreeMap<String, Value>,
) -> crate::evidence::BenchmarkObservation {
    let (family, variant, version, version_order, task_count, comparable) = match field {
        "terminalbenchV40" => (
            "terminal",
            "Terminal-Bench",
            "v4.0",
            vec![4, 0],
            Some(66),
            "terminal-bench",
        ),
        "terminalbenchV21" => (
            "terminal",
            "Terminal-Bench",
            "v2.1",
            vec![2, 1],
            Some(89),
            "terminal-bench",
        ),
        "gpqa" => (
            "gpqa",
            "GPQA Diamond",
            "legacy",
            vec![0],
            Some(198),
            "gpqa-diamond-aa",
        ),
        "hle" => (
            "hle",
            "text-only",
            "unknown",
            vec![0],
            Some(2158),
            "hle-text-only-aa",
        ),
        "lcr" => ("lcr", "AA-LCR", "unknown", vec![0], Some(100), "aa-lcr"),
        value if value.starts_with("omniscience") => (
            "omniscience",
            "full-private",
            "unknown",
            vec![0],
            Some(6000),
            "aa-omniscience-full",
        ),
        "intelligenceIndex" => (
            "intelligence-index",
            "Artificial Analysis",
            "unknown",
            vec![0],
            None,
            "aa-intelligence-index",
        ),
        value if value.starts_with("gdpPdf") => (
            "gdp-pdf",
            "Artificial Analysis implementation",
            "unknown",
            vec![0],
            Some(100),
            "aa-gdp-pdf",
        ),
        value => (value, value, "unknown", vec![0], None, value),
    };
    let bounds = match family {
        "terminal" | "gpqa" | "hle" | "lcr" | "omniscience" | "gdp-pdf" => (Some(0.0), Some(1.0)),
        "intelligence-index" => (Some(0.0), Some(100.0)),
        _ => (None, None),
    };
    crate::evidence::BenchmarkObservation {
        observation_id: format!("aa:{}:{}:{field}", source.revision, subject.raw_model_id),
        source: source.clone(),
        series: crate::evidence::BenchmarkSeries {
            family: family.to_owned(),
            variant: variant.to_owned(),
            version: version.to_owned(),
            version_order,
            task_count,
            comparable_series_id: comparable.to_owned(),
        },
        subject: subject.clone(),
        execution: crate::evidence::ExecutionIdentity {
            harness: "Artificial Analysis model evaluation".to_owned(),
            harness_version: None,
            protocol: "See source methodology".to_owned(),
            grader: "See source methodology".to_owned(),
        },
        score: Some(crate::evidence::ScoreObservation {
            metric: field.to_owned(),
            value,
            scale_min: bounds.0,
            scale_max: bounds.1,
            higher_is_better: true,
        }),
        resources: None,
        extra,
    }
}

fn evidence_catalog() -> EvidenceCatalog {
    let snapshots = SNAPSHOTS
        .get_or_init(|| RwLock::new(Vec::new()))
        .read()
        .expect("upstream snapshot lock");
    EvidenceCatalog {
        sources: snapshots
            .iter()
            .map(|snapshot| SourceStatus {
                source: snapshot.source.clone(),
                fetch_error: snapshot.fetch_error.clone(),
                last_good_revision: snapshot.last_good_revision.clone(),
                last_good_fetched_at: snapshot.last_good_fetched_at.clone(),
            })
            .collect(),
        observations: snapshots
            .iter()
            .flat_map(|snapshot| snapshot.observations.iter().cloned())
            .collect(),
    }
}

pub fn project(rows: &mut Vec<Row>, allow_cross_harness_proxy: bool) -> EvidenceCatalog {
    let EvidenceCatalog {
        sources,
        mut observations,
    } = evidence_catalog();
    observations.extend(row_observations(rows, &sources));
    enrich_aa_resources(rows, &mut observations);
    let model_rows: Vec<_> = rows
        .iter()
        .filter(|row| row.harness == "model")
        .cloned()
        .collect();
    let model_rows_by_subject: BTreeMap<_, _> = model_rows
        .iter()
        .map(|row| (crate::evidence::subject_key(&row_subject(row)), row.clone()))
        .collect();
    let native_rows_by_model: BTreeMap<_, Vec<_>> = rows
        .iter()
        .filter(|row| {
            crate::subscriptions::PROVIDERS.iter().any(|provider| {
                provider.id == row.vendor
                    && provider
                        .plans
                        .iter()
                        .any(|plan| crate::subscriptions::eligible(provider, plan, row))
            })
        })
        .fold(BTreeMap::new(), |mut by_model, row| {
            by_model
                .entry(crate::evidence::model_key(&row_subject(row)))
                .or_default()
                .push(row.clone());
            by_model
        });
    for observation in &observations {
        let Some(model_row) =
            model_rows_by_subject.get(&crate::evidence::subject_key(&observation.subject))
        else {
            continue;
        };
        let Some(native_rows) =
            native_rows_by_model.get(&crate::evidence::model_key(&observation.subject))
        else {
            continue;
        };
        for native_row in native_rows {
            let harness = native_row.harness.clone();
            let exact_source = observation.execution.harness == harness;
            let model_level = observation
                .execution
                .harness
                .eq_ignore_ascii_case("Artificial Analysis model evaluation")
                && crate::evidence::is_model_level_family(&observation.series.family);
            if (!exact_source && !model_level && !allow_cross_harness_proxy)
                || rows.iter().any(|row| {
                    row.harness == harness
                        && crate::evidence::same_subject(&row_subject(row), &observation.subject)
                })
            {
                continue;
            }
            let mut row = model_row.clone();
            row.harness = harness.clone();
            row.family = crate::aa::family_key(&harness, &row.model_key);
            row.display_name = format!(
                "{} - {}{}{}",
                harness,
                row.model,
                observation
                    .subject
                    .effort
                    .as_deref()
                    .map(|value| format!(" ({value})"))
                    .unwrap_or_default(),
                if exact_source {
                    ""
                } else if model_level {
                    " (model-level evidence)"
                } else {
                    " (benchmark proxy)"
                }
            );
            row.benchmark_observation_ids.clear();
            row.benchmark_evidence.clear();
            rows.push(row);
        }
    }
    let observations_by_subject: BTreeMap<_, Vec<_>> =
        observations
            .iter()
            .fold(BTreeMap::new(), |mut by_subject, observation| {
                by_subject
                    .entry(crate::evidence::subject_key(&observation.subject))
                    .or_default()
                    .push(observation.clone());
                by_subject
            });
    for row in rows.iter_mut() {
        let subject = row_subject(row);
        let matched = observations_by_subject
            .get(&crate::evidence::subject_key(&subject))
            .cloned()
            .unwrap_or_default();
        row.benchmark_observation_ids = matched
            .iter()
            .map(|observation| observation.observation_id.clone())
            .collect();
        let mut series = std::collections::BTreeSet::new();
        row.benchmark_evidence.clear();
        for observation in &matched {
            let key = (
                observation.series.family.clone(),
                observation.series.comparable_series_id.clone(),
            );
            if !is_role_series(&key.0, &key.1) {
                continue;
            }
            if !series.insert(key.clone()) {
                continue;
            }
            let expected = crate::evidence::ExecutionIdentity {
                harness: row.harness.clone(),
                harness_version: None,
                protocol: String::new(),
                grader: String::new(),
            };
            if let Some(selected) = crate::evidence::select_latest(
                matched.as_slice(),
                &subject,
                &expected,
                &key.0,
                &key.1,
                allow_cross_harness_proxy,
            ) {
                row.benchmark_evidence.push(selected.projection());
            }
        }
        apply_selected(row);
    }
    EvidenceCatalog {
        sources,
        observations,
    }
}

fn row_observations(
    rows: &[Row],
    sources: &[SourceStatus],
) -> Vec<crate::evidence::BenchmarkObservation> {
    rows.iter()
        .flat_map(|row| {
            let source_id = if row.harness == "model" {
                "artificial-analysis-model-manifest"
            } else {
                "artificial-analysis-coding-agents"
            };
            let source_status = sources
                .iter()
                .find(|status| status.source.source_id == source_id);
            let fetched_at = source_status
                .map(|status| status.source.fetched_at.clone())
                .unwrap_or_default();
            let source_revision = source_status
                .map(|status| status.source.revision.clone())
                .unwrap_or_else(|| "unknown".to_owned());
            row.task_metrics.iter().filter_map(move |metric| {
                if row.harness != "model"
                    && !matches!(
                        metric.benchmark,
                        Benchmark::Swe | Benchmark::Terminal | Benchmark::Qna
                    )
                {
                    return None;
                }
                let (family, variant, version, order, comparable) = metric_series(metric);
                let identity = format!(
                    "{}:{}:{}:{}:{}:{:.12}:{:.12}:{:.12}:{:.12}",
                    row.harness,
                    row.model_key,
                    metric.dataset_id,
                    metric.time_basis,
                    metric.cost_basis,
                    metric.pass,
                    metric.seconds,
                    metric.pooled_seconds,
                    metric.usd
                );
                let observation_digest = hex::encode(Sha256::digest(identity.as_bytes()));
                let source = SourceRef {
                    source_id: source_id.to_owned(),
                    url: if row.harness == "model" {
                        "https://artificialanalysis.ai/evaluations/livecodebench".to_owned()
                    } else {
                        "https://artificialanalysis.ai/agents/coding-agents".to_owned()
                    },
                    revision: format!("{source_revision}:normalized-rows"),
                    fetched_at: fetched_at.clone(),
                    observed_at: None,
                    published_at: None,
                    license: None,
                };
                Some(crate::evidence::BenchmarkObservation {
                    observation_id: format!("aa-row:{observation_digest}"),
                    source,
                    series: crate::evidence::BenchmarkSeries {
                        family,
                        variant,
                        version,
                        version_order: order,
                        task_count: Some(metric.task_count),
                        comparable_series_id: comparable,
                    },
                    subject: row_subject(row),
                    execution: crate::evidence::ExecutionIdentity {
                        harness: if row.harness == "model" {
                            "Artificial Analysis model evaluation".to_owned()
                        } else {
                            row.harness.clone()
                        },
                        harness_version: None,
                        protocol: metric.dataset_id.clone(),
                        grader: "Published benchmark protocol".to_owned(),
                    },
                    score: Some(crate::evidence::ScoreObservation {
                        metric: primary_metric(metric.benchmark).to_owned(),
                        value: metric.pass,
                        scale_min: Some(0.0),
                        scale_max: Some(1.0),
                        higher_is_better: true,
                    }),
                    resources: (row.harness != "model" || metric.canonical_resources).then(|| {
                        crate::evidence::ResourceObservation {
                            seconds: Some(metric.seconds),
                            pooled_seconds: Some(metric.pooled_seconds),
                            usd: Some(metric.usd),
                            input_tokens: None,
                            output_tokens: None,
                            time_basis: metric.time_basis.clone(),
                            cost_basis: metric.cost_basis.clone(),
                            canonical_resources: metric.canonical_resources,
                        }
                    }),
                    extra: if metric.benchmark == Benchmark::Omniscience {
                        BTreeMap::from([
                            ("accuracy".to_owned(), Value::from(row.omniscience_accuracy)),
                            (
                                "attemptRate".to_owned(),
                                Value::from(row.omniscience_attempt_rate),
                            ),
                            ("hallucinationRate".to_owned(), Value::from(row.halluc)),
                        ])
                    } else {
                        BTreeMap::new()
                    },
                })
            })
        })
        .collect()
}

fn metric_series(metric: &TaskMetric) -> (String, String, String, Vec<u64>, String) {
    let (family, variant, version, order, comparable) = match metric.dataset_id.as_str() {
        "deep-swe-v1.1" => ("swe", "DeepSWE", "v1.1", vec![1, 1], "deep-swe"),
        "deep-swe-v1" => ("swe", "DeepSWE", "v1", vec![1, 0], "deep-swe"),
        "terminal-bench-v4" => (
            "terminal",
            "Terminal-Bench",
            "v4.0",
            vec![4, 0],
            "terminal-bench",
        ),
        "terminal-bench-v2.1" => (
            "terminal",
            "Terminal-Bench",
            "v2.1",
            vec![2, 1],
            "terminal-bench",
        ),
        "gpqa" => ("gpqa", "GPQA Diamond", "legacy", vec![0], "gpqa-diamond-aa"),
        "hle" => ("hle", "text-only", "unknown", vec![0], "hle-text-only-aa"),
        "lcr" => ("lcr", "AA-LCR", "unknown", vec![0], "aa-lcr"),
        "omniscience" => (
            "omniscience",
            "full-private",
            "unknown",
            vec![0],
            "aa-omniscience-full",
        ),
        _ => (
            benchmark_family(metric.benchmark),
            metric.dataset_id.as_str(),
            "unknown",
            vec![0],
            metric.dataset_id.as_str(),
        ),
    };
    (
        family.to_owned(),
        variant.to_owned(),
        version.to_owned(),
        order,
        comparable.to_owned(),
    )
}

fn benchmark_family(benchmark: Benchmark) -> &'static str {
    match benchmark {
        Benchmark::Swe => "swe",
        Benchmark::Terminal => "terminal",
        Benchmark::Qna => "qna",
        Benchmark::Gpqa => "gpqa",
        Benchmark::Hle => "hle",
        Benchmark::Lcr => "lcr",
        Benchmark::Omniscience => "omniscience",
    }
}

fn primary_metric(benchmark: Benchmark) -> &'static str {
    match benchmark {
        Benchmark::Swe => "pass@1",
        Benchmark::Terminal | Benchmark::Qna => "accuracy",
        Benchmark::Gpqa => "gpqa",
        Benchmark::Hle => "hle",
        Benchmark::Lcr => "lcr",
        Benchmark::Omniscience => "omniscience.accuracy",
    }
}

fn is_role_series(family: &str, comparable_series_id: &str) -> bool {
    matches!(
        (family, comparable_series_id),
        ("swe", "deep-swe")
            | ("terminal", "terminal-bench")
            | ("qna", "swe-atlas-qna")
            | ("gpqa", "gpqa-diamond-aa")
            | ("hle", "hle-text-only-aa")
            | ("lcr", "aa-lcr")
            | ("omniscience", "aa-omniscience-full")
    )
}

fn apply_selected(row: &mut Row) {
    for projection in &row.benchmark_evidence {
        if !is_role_projection(projection) {
            continue;
        }
        let Some(value) = projection.score.normalized_value() else {
            continue;
        };
        let benchmark = match projection.family.as_str() {
            "swe" => {
                row.swe = Some(value);
                Some(Benchmark::Swe)
            }
            "terminal" => {
                row.term = Some(value);
                Some(Benchmark::Terminal)
            }
            "qna" => {
                row.qna = Some(value);
                Some(Benchmark::Qna)
            }
            "gpqa" => {
                row.gpqa = Some(value);
                Some(Benchmark::Gpqa)
            }
            "hle" => {
                row.hle = Some(value);
                Some(Benchmark::Hle)
            }
            "lcr" => {
                row.lcr = Some(value);
                Some(Benchmark::Lcr)
            }
            "omniscience" if projection.score.metric == "omniscience.accuracy" => {
                row.omniscience_accuracy = Some(value);
                Some(Benchmark::Omniscience)
            }
            _ => None,
        };
        let Some(benchmark) = benchmark else {
            continue;
        };
        row.task_metrics
            .retain(|metric| metric.benchmark != benchmark);
        let Some(resources) = &projection.resources else {
            continue;
        };
        let Some(seconds) = resources
            .seconds
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            continue;
        };
        let Some(usd) = resources
            .usd
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            continue;
        };
        row.task_metrics.push(TaskMetric {
            benchmark,
            dataset_id: canonical_dataset_id(projection),
            task_count: projection.series.task_count.unwrap_or_default(),
            pass: value,
            seconds,
            pooled_seconds: resources.pooled_seconds.unwrap_or(seconds),
            usd,
            time_basis: format!(
                "{}; source harness {} ({:?})",
                resources.time_basis, projection.execution.harness, projection.transfer
            ),
            cost_basis: format!(
                "{}; source {} revision {}",
                resources.cost_basis, projection.source.source_id, projection.source.revision
            ),
            canonical_resources: resources.canonical_resources,
        });
    }
}

fn is_role_projection(projection: &crate::evidence::EvidenceProjection) -> bool {
    matches!(
        (
            projection.family.as_str(),
            projection.series.comparable_series_id.as_str(),
            projection.score.metric.as_str()
        ),
        ("swe", "deep-swe", "pass@1")
            | (
                "terminal",
                "terminal-bench",
                "accuracy" | "terminalbenchV40" | "terminalbenchV21"
            )
            | ("qna", "swe-atlas-qna", "accuracy")
            | ("gpqa", "gpqa-diamond-aa", "gpqa")
            | ("hle", "hle-text-only-aa", "hle")
            | ("lcr", "aa-lcr", "lcr")
            | ("omniscience", "aa-omniscience-full", "omniscience.accuracy")
    )
}

fn canonical_dataset_id(projection: &crate::evidence::EvidenceProjection) -> String {
    match (
        projection.family.as_str(),
        projection.series.version.as_str(),
    ) {
        ("swe", "v1.1") => "deep-swe-v1.1".to_owned(),
        ("swe", "v1") => "deep-swe-v1".to_owned(),
        ("terminal", "v4.0" | "v4") => "terminal-bench-v4".to_owned(),
        ("terminal", "v2.1") => "terminal-bench-v2.1".to_owned(),
        _ => projection.series.comparable_series_id.clone(),
    }
}

fn enrich_aa_resources(rows: &[Row], observations: &mut [crate::evidence::BenchmarkObservation]) {
    for observation in observations.iter_mut().filter(|observation| {
        observation.source.source_id == "artificial-analysis-model-manifest"
            && observation.resources.is_none()
    }) {
        let Some(row) = rows.iter().find(|row| {
            row.harness == "model"
                && crate::evidence::same_subject(&row_subject(row), &observation.subject)
        }) else {
            continue;
        };
        let Some(benchmark) = benchmark_for_family(&observation.series.family) else {
            continue;
        };
        let Some(metric) = row.task_metrics.iter().find(|metric| {
            metric.benchmark == benchmark
                && metric.dataset_id == observation_dataset_id(observation)
                && metric.canonical_resources
        }) else {
            continue;
        };
        observation.resources = Some(crate::evidence::ResourceObservation {
            seconds: Some(metric.seconds),
            pooled_seconds: Some(metric.pooled_seconds),
            usd: Some(metric.usd),
            input_tokens: None,
            output_tokens: None,
            time_basis: metric.time_basis.clone(),
            cost_basis: metric.cost_basis.clone(),
            canonical_resources: metric.canonical_resources,
        });
    }
}

fn observation_dataset_id(observation: &crate::evidence::BenchmarkObservation) -> String {
    match (
        observation.series.family.as_str(),
        observation.series.version.as_str(),
    ) {
        ("terminal", "v4.0") => "terminal-bench-v4".to_owned(),
        ("terminal", "v2.1") => "terminal-bench-v2.1".to_owned(),
        ("gpqa", _) => "gpqa".to_owned(),
        ("hle", _) => "hle".to_owned(),
        ("lcr", _) => "lcr".to_owned(),
        ("omniscience", _) => "omniscience".to_owned(),
        _ => observation.series.comparable_series_id.clone(),
    }
}

fn benchmark_for_family(family: &str) -> Option<Benchmark> {
    match family {
        "swe" => Some(Benchmark::Swe),
        "terminal" => Some(Benchmark::Terminal),
        "qna" => Some(Benchmark::Qna),
        "gpqa" => Some(Benchmark::Gpqa),
        "hle" => Some(Benchmark::Hle),
        "lcr" => Some(Benchmark::Lcr),
        "omniscience" => Some(Benchmark::Omniscience),
        _ => None,
    }
}

fn row_subject(row: &Row) -> crate::evidence::ModelSubject {
    crate::evidence::ModelSubject {
        provider: row.vendor.clone(),
        model: row.model.clone(),
        raw_model_id: row.model_key.clone(),
        effort: row.effort.clone(),
    }
}
