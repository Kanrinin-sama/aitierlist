use anyhow::{Context, Result, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::evidence::{
    BenchmarkObservation, BenchmarkSeries, ExecutionIdentity, ModelSubject, ResourceObservation,
    ScoreObservation, SourceRef, SourceSnapshot,
};

pub fn parse(source: SourceRef, payload: Value) -> Result<SourceSnapshot> {
    let (family, variant, version, version_order, task_count, protocol) = version(&source)?;
    let submissions = if let Some(values) = payload.as_array() {
        values.iter().collect::<Vec<_>>()
    } else if let Some(values) = payload["submissions"].as_array() {
        values.iter().collect()
    } else {
        vec![payload.get("submission").unwrap_or(&payload)]
    };
    let mut observations = Vec::with_capacity(submissions.len());
    for submission in submissions {
        let metadata = submission["metadata"]
            .as_object()
            .context("Terminal-Bench submission metadata is unavailable")?;
        let metrics = submission["metrics"]
            .as_object()
            .context("Terminal-Bench submission metrics are unavailable")?;
        let harness_label = nested_text(metadata, "agent_display", "label")?;
        let harness = submission["source_filter"]["agent_name"]
            .as_str()
            .filter(|value| !value.is_empty())
            .unwrap_or(harness_label);
        let model = nested_text(metadata, "model_display", "label")?;
        let raw_model_id = submission["source_filter"]["model_name"]
            .as_str()
            .unwrap_or(model);
        let provider = nested_text(metadata, "model_org", "label").unwrap_or_default();
        let effort = metadata
            .get("reasoning_effort")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let accuracy = metrics
            .get("accuracy")
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite());
        let n_trials = metrics.get("n_trials").and_then(Value::as_f64);
        let resources = resource(metrics, n_trials);
        anyhow::ensure!(
            accuracy.is_some() || resources.is_some(),
            "Terminal-Bench submission has no score or resources"
        );
        let mut extra = object_extra(&Value::Object(metrics.clone()));
        extra.insert("source_jobs".to_owned(), submission["source_jobs"].clone());
        extra.insert("trials".to_owned(), submission["trials"].clone());
        extra.insert(
            "disqualified_trials".to_owned(),
            submission["disqualified_trials"].clone(),
        );
        extra.insert(
            "source_filter".to_owned(),
            submission["source_filter"].clone(),
        );
        extra.insert("metadata".to_owned(), Value::Object(metadata.clone()));
        extra.insert(
            "harness_display".to_owned(),
            Value::String(harness_label.to_owned()),
        );
        if let Some(host) = raw_model_id.split_once('/').map(|(host, _)| host) {
            extra.insert("evaluation_host".to_owned(), Value::String(host.to_owned()));
        }
        if let Some(run) = payload.get("run") {
            extra.insert("run".to_owned(), run.clone());
        }
        for key in ["_submission_path", "_submission_url"] {
            if let Some(value) = submission.get(key) {
                extra.insert(key.trim_start_matches('_').to_owned(), value.clone());
            }
        }
        let published_identity = Sha256::digest(serde_json::to_vec(submission)?);
        let identity = format!(
            "{}:{}:{}:{}:{}:{}",
            source.source_id,
            source.revision,
            harness,
            raw_model_id,
            effort.as_deref().unwrap_or("none"),
            hex::encode(&published_identity[..12])
        );
        observations.push(BenchmarkObservation {
            observation_id: identity,
            source: source.clone(),
            series: BenchmarkSeries {
                family: family.to_owned(),
                variant: variant.to_owned(),
                version: version.to_owned(),
                version_order: version_order.clone(),
                task_count: Some(task_count),
                comparable_series_id: if family == "terminal" {
                    "terminal-bench".to_owned()
                } else {
                    family.to_owned()
                },
            },
            subject: ModelSubject {
                provider: provider.to_owned(),
                model: model.to_owned(),
                raw_model_id: raw_model_id.to_owned(),
                effort,
            },
            execution: ExecutionIdentity {
                harness: harness.to_owned(),
                harness_version: submission["source_filter"]["agent_version"]
                    .as_str()
                    .map(str::to_owned),
                protocol: protocol.to_owned(),
                grader: "Terminal-Bench task verifiers".to_owned(),
            },
            score: accuracy.map(|value| ScoreObservation {
                metric: "accuracy".to_owned(),
                value,
                scale_min: Some(0.0),
                scale_max: Some(1.0),
                higher_is_better: true,
            }),
            resources,
            extra,
        });
    }
    Ok(SourceSnapshot {
        source,
        payload: Some(payload),
        observations,
        fetch_error: None,
        last_good_revision: None,
        last_good_fetched_at: None,
    })
}

type TerminalVersion = (
    &'static str,
    &'static str,
    &'static str,
    Vec<u64>,
    usize,
    &'static str,
);

fn version(source: &SourceRef) -> Result<TerminalVersion> {
    let identity =
        format!("{} {} {}", source.source_id, source.url, source.revision).to_lowercase();
    if identity.contains("science") {
        Ok((
            "terminal-science",
            "Terminal-Bench Science",
            "v0.1",
            vec![0, 1],
            70,
            "Harbor Terminal-Bench Science v0.1",
        ))
    } else if identity.contains("2-1") || identity.contains("2.1") {
        Ok((
            "terminal",
            "Terminal-Bench",
            "v2.1",
            vec![2, 1],
            89,
            "Harbor Terminal-Bench v2.1",
        ))
    } else if identity.contains("v4")
        || identity.contains("4.0")
        || identity.contains("terminal-bench-v4")
    {
        Ok((
            "terminal",
            "Terminal-Bench",
            "v4.0",
            vec![4, 0],
            66,
            "Harbor Terminal-Bench v4.0.0",
        ))
    } else {
        bail!("Terminal-Bench source version is unavailable")
    }
}

fn nested_text<'a>(
    object: &'a serde_json::Map<String, Value>,
    key: &str,
    nested: &str,
) -> Result<&'a str> {
    object
        .get(key)
        .and_then(|value| value.get(nested))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Terminal-Bench metadata has no {key}.{nested}"))
}

fn resource(
    metrics: &serde_json::Map<String, Value>,
    n_trials: Option<f64>,
) -> Option<ResourceObservation> {
    let divisor = n_trials.filter(|value| value.is_finite() && *value > 0.0);
    let per_trial = |key: &str| {
        metrics
            .get(key)
            .and_then(Value::as_f64)
            .filter(|value| value.is_finite())
            .zip(divisor)
            .map(|(value, divisor)| value / divisor)
    };
    let seconds = metrics
        .get("avg_trial_duration_sec")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite());
    let usd = per_trial("total_cost_usd");
    let input_tokens = per_trial("uncached_input_tokens")
        .zip(per_trial("cached_input_tokens"))
        .map(|(uncached, cached)| uncached + cached);
    let output_tokens = per_trial("output_tokens");
    (seconds.is_some() || usd.is_some() || input_tokens.is_some() || output_tokens.is_some())
        .then_some(ResourceObservation {
            seconds,
            pooled_seconds: None,
            usd,
            input_tokens,
            output_tokens,
            time_basis: "Published Terminal-Bench trial mean duration".to_owned(),
            cost_basis: "Published Terminal-Bench submission total cost divided by n_trials"
                .to_owned(),
            canonical_resources: false,
        })
}

fn object_extra(value: &Value) -> BTreeMap<String, Value> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}
