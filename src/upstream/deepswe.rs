use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::evidence::{
    BenchmarkObservation, BenchmarkSeries, ExecutionIdentity, ModelSubject, ResourceObservation,
    ScoreObservation, SourceRef, SourceSnapshot,
};

pub fn parse(source: SourceRef, payload: Value) -> Result<SourceSnapshot> {
    let dataset = dataset(&source)?;
    let rows = payload["rows"]
        .as_array()
        .context("DeepSWE rows are unavailable")?;
    let task_count = payload["n_tasks_in_set"]
        .as_u64()
        .or_else(|| payload["n_tasks"].as_u64())
        .and_then(|n| usize::try_from(n).ok())
        .or(dataset.task_count);
    let observations = if payload.get("n_trials").is_some()
        || rows.first().is_some_and(|r| r.get("trial_name").is_some())
        || payload.get("n_tasks").is_some()
        || rows
            .first()
            .is_some_and(|r| r.get("base_commit_hash").is_some())
    {
        Vec::new()
    } else {
        rows.iter()
            .map(|r| aggregate(&source, dataset, task_count, r))
            .collect::<Result<Vec<_>>>()?
    };
    Ok(SourceSnapshot {
        source,
        payload: Some(payload),
        observations,
        fetch_error: None,
        last_good_revision: None,
        last_good_fetched_at: None,
    })
}

#[derive(Clone, Copy)]
struct Dataset {
    version: &'static str,
    order: &'static [u64],
    task_count: Option<usize>,
    protocol: &'static str,
}

struct RowObservation<'a> {
    identity: String,
    model: &'a str,
    score: Option<ScoreObservation>,
    resources: Option<ResourceObservation>,
}

fn dataset(source: &SourceRef) -> Result<Dataset> {
    let id = format!("{} {} {}", source.source_id, source.url, source.revision).to_lowercase();
    if id.contains("v1.1") {
        return Ok(Dataset {
            version: "v1.1",
            order: &[1, 1],
            task_count: Some(113),
            protocol: "Pier DeepSWE v1.1",
        });
    }
    if id.contains("/v1/") || id.contains("deep-swe-v1") || id.contains("deepswe-v1") {
        return Ok(Dataset {
            version: "v1",
            order: &[1, 0],
            task_count: None,
            protocol: "Pier DeepSWE v1",
        });
    }
    anyhow::bail!("DeepSWE source does not identify a supported dataset version")
}

fn aggregate(
    source: &SourceRef,
    dataset: Dataset,
    task_count: Option<usize>,
    row: &Value,
) -> Result<BenchmarkObservation> {
    let model = text(row, "model")?;
    let config = text(row, "config")?;
    let score = finite(row, "pass_at_1").map(|value| score("pass@1", value));
    let resources = resource(row);
    anyhow::ensure!(
        score.is_some() || resources.is_some(),
        "DeepSWE row has no score or resources"
    );
    build(
        source,
        dataset,
        task_count,
        row,
        RowObservation {
            identity: format!("aggregate:{config}"),
            model,
            score,
            resources,
        },
    )
}

fn build(
    source: &SourceRef,
    dataset: Dataset,
    task_count: Option<usize>,
    row: &Value,
    observation: RowObservation<'_>,
) -> Result<BenchmarkObservation> {
    let harness = text(row, "harness")?;
    let mut extra = object_extra(row);
    for key in [
        "model",
        "harness",
        "reasoning_effort",
        "config",
        "pass_at_1",
        "score_value",
        "reward",
    ] {
        extra.remove(key);
    }
    if let Some(host) = row["provider"].as_str().filter(|v| !v.is_empty()) {
        extra.insert("evaluation_host".to_owned(), Value::String(host.to_owned()));
    }
    Ok(BenchmarkObservation {
        observation_id: format!(
            "deep-swe:{}:{}:{}",
            dataset.version, source.revision, observation.identity
        ),
        source: source.clone(),
        series: BenchmarkSeries {
            family: "swe".to_owned(),
            variant: "DeepSWE".to_owned(),
            version: dataset.version.to_owned(),
            version_order: dataset.order.to_vec(),
            task_count,
            comparable_series_id: "deep-swe".to_owned(),
        },
        subject: ModelSubject {
            provider: model_provider(observation.model).to_owned(),
            model: observation.model.to_owned(),
            raw_model_id: observation.model.to_owned(),
            effort: row["reasoning_effort"]
                .as_str()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_owned),
        },
        execution: ExecutionIdentity {
            harness: harness.to_owned(),
            harness_version: None,
            protocol: dataset.protocol.to_owned(),
            grader: "DeepSWE task verifier".to_owned(),
        },
        score: observation.score,
        resources: observation.resources,
        extra,
    })
}

fn model_provider(model: &str) -> &str {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("claude-") {
        "anthropic"
    } else if lower.starts_with("gemini-") {
        "google"
    } else if lower.starts_with("gpt-") {
        "openai"
    } else if lower.starts_with("grok-") {
        "xai"
    } else if lower.starts_with("glm-") {
        "zai"
    } else if lower.starts_with("kimi-") {
        "moonshot"
    } else if lower.starts_with("muse-") {
        "muse"
    } else if lower.starts_with("qwen") {
        "alibaba"
    } else if lower.starts_with("deepseek-") {
        "deepseek"
    } else if lower.starts_with("minimax-") {
        "minimax"
    } else if lower.starts_with("mimo-") {
        "xiaomi"
    } else {
        ""
    }
}

fn score(metric: &str, value: f64) -> ScoreObservation {
    ScoreObservation {
        metric: metric.to_owned(),
        value,
        scale_min: Some(0.0),
        scale_max: Some(1.0),
        higher_is_better: true,
    }
}
fn text<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .with_context(|| format!("DeepSWE row has no {key}"))
}
fn finite(value: &Value, key: &str) -> Option<f64> {
    value[key].as_f64().filter(|v| v.is_finite())
}

fn resource(row: &Value) -> Option<ResourceObservation> {
    let seconds = finite(row, "mean_duration_seconds");
    let usd = finite(row, "mean_cost_usd");
    let input_tokens = finite(row, "mean_input_tokens");
    let output_tokens = finite(row, "mean_output_tokens");
    (seconds.is_some() || usd.is_some() || input_tokens.is_some() || output_tokens.is_some())
        .then_some(ResourceObservation {
            seconds,
            pooled_seconds: None,
            usd,
            input_tokens,
            output_tokens,
            time_basis: "Published mean duration over included scored DeepSWE rollout attempts"
                .to_owned(),
            cost_basis: "Published mean cost over included scored DeepSWE rollout attempts"
                .to_owned(),
            canonical_resources: false,
        })
}

fn object_extra(value: &Value) -> BTreeMap<String, Value> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}
