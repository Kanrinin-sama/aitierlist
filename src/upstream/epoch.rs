use crate::evidence::{
    BenchmarkObservation, BenchmarkSeries, ExecutionIdentity, ModelSubject, ScoreObservation,
    SourceRef, SourceSnapshot,
};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub fn parse(source: SourceRef, payload: Value) -> Result<SourceSnapshot> {
    let metadata = payload
        .get("metadata")
        .and_then(Value::as_array)
        .context("Epoch payload metadata is unavailable")?;
    let files = payload
        .get("files")
        .and_then(Value::as_object)
        .context("Epoch payload files are unavailable")?;
    let mut observations = Vec::new();
    let mut covered = std::collections::BTreeSet::new();
    for definition in metadata.iter().filter_map(Value::as_object) {
        let Some(file) = text(definition, &["source_file", "sourceFile"]) else {
            continue;
        };
        let Some(rows) = files.get(file).and_then(Value::as_array) else {
            continue;
        };
        covered.insert(file);
        for row in rows.iter().filter_map(Value::as_object) {
            if let Some(observation) = observation(&source, definition, file, row) {
                observations.push(observation);
            }
        }
    }
    for (file, rows) in files {
        if covered.contains(file.as_str()) {
            continue;
        }
        let Some(rows) = rows.as_array() else {
            continue;
        };
        let mut definition = Map::new();
        definition.insert(
            "benchmark".to_owned(),
            Value::String(file.trim_end_matches(".csv").replace('_', " ")),
        );
        definition.insert(
            "score_column".to_owned(),
            Value::String(score_column(file).unwrap_or("__unavailable__").to_owned()),
        );
        for row in rows.iter().filter_map(Value::as_object) {
            if let Some(observation) = observation(&source, &definition, file, row) {
                observations.push(observation);
            }
        }
    }
    if observations.is_empty() {
        bail!("Epoch archive payload has no usable observations")
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

fn observation(
    source: &SourceRef,
    definition: &Map<String, Value>,
    file: &str,
    row: &Map<String, Value>,
) -> Option<BenchmarkObservation> {
    let benchmark = text(definition, &["benchmark"])?;
    let score_column = text(definition, &["score_column", "scoreColumn"])?;
    let score = row
        .get(score_column)
        .and_then(scalar)
        .filter(|value| value.is_finite());
    let model = text(row, &["Model version", "model", "Model"])?;
    let raw_model_id = text(row, &["Model version", "model_id", "Model ID"]).unwrap_or(model);
    let effort = text(row, &["reasoning_effort", "Reasoning effort", "effort"])
        .map(str::to_owned)
        .or_else(|| {
            model
                .rsplit_once('_')
                .and_then(|(_, value)| effort(value))
                .map(str::to_owned)
        });
    let provider = text(row, &["Organization", "organization", "provider"]).unwrap_or("unknown");
    let harness =
        text(row, &["Harness", "harness", "Agent", "agent"]).unwrap_or("Epoch evaluation");
    let protocol = text(row, &["Scorer", "scorer", "protocol"]).unwrap_or("Epoch published run");
    let grader = text(row, &["Judge", "judge", "grader"]).unwrap_or("unspecified");
    let family = family(benchmark);
    let version = text(
        definition,
        &["benchmark_version", "benchmarkVersion", "version"],
    )
    .unwrap_or("");
    let comparable_series_id = format!("epoch:{family}:{file}:{protocol}:{grader}");
    let mut observation_source = source.clone();
    if let Some(started) = text(row, &["Started at", "started_at", "startedAt"]) {
        observation_source.observed_at = Some(started.to_owned());
    }
    let mut extra: BTreeMap<_, _> = row
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in definition {
        extra.insert(format!("metadata.{key}"), value.clone());
    }
    extra.insert("sourceFile".to_owned(), Value::String(file.to_owned()));
    Some(BenchmarkObservation {
        observation_id: format!(
            "{}:{file}:{}",
            source.revision,
            row.get("id")
                .and_then(Value::as_str)
                .unwrap_or(raw_model_id)
        ),
        source: observation_source,
        series: BenchmarkSeries {
            family: family.clone(),
            variant: benchmark.to_owned(),
            version: version.to_owned(),
            version_order: version_order(version),
            task_count: integer(
                definition
                    .get("task_count")
                    .or_else(|| definition.get("taskCount")),
            ),
            comparable_series_id,
        },
        subject: ModelSubject {
            provider: provider.to_owned(),
            model: model_name(model, effort.as_deref()),
            raw_model_id: raw_model_id.to_owned(),
            effort,
        },
        execution: ExecutionIdentity {
            harness: harness.to_owned(),
            harness_version: text(row, &["Harness version", "harness_version"]).map(str::to_owned),
            protocol: protocol.to_owned(),
            grader: grader.to_owned(),
        },
        score: score.map(|value| ScoreObservation {
            metric: score_column.to_owned(),
            value,
            scale_min: matches!(family.as_str(), "gpqa" | "hle").then_some(0.0),
            scale_max: matches!(family.as_str(), "gpqa" | "hle").then_some(1.0),
            higher_is_better: true,
        }),
        resources: None,
        extra,
    })
}

fn family(value: &str) -> String {
    let normalized = normalize(value)
        .trim_end_matches("-external")
        .trim_end_matches("-internal")
        .to_owned();
    if normalized == "gpqa-diamond" {
        "gpqa".to_owned()
    } else if normalized.starts_with("humanity-s-last-exam") || normalized.starts_with("hle") {
        "hle".to_owned()
    } else {
        normalized
    }
}

fn model_name(model: &str, effort: Option<&str>) -> String {
    effort
        .and_then(|effort| model.strip_suffix(&format!("_{effort}")))
        .unwrap_or(model)
        .replace('_', " ")
}

fn effort(value: &str) -> Option<&str> {
    ["none", "minimal", "low", "medium", "high", "xhigh", "max"]
        .contains(&value.to_ascii_lowercase().as_str())
        .then_some(value)
}

fn text<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key)?.as_str())
        .filter(|value| !value.is_empty())
}

fn scalar(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.replace('%', "").parse().ok())
}

fn integer(value: Option<&Value>) -> Option<usize> {
    value
        .and_then(scalar)
        .map(|value| value as usize)
        .filter(|value| *value > 0)
}

fn normalize(value: &str) -> String {
    Regex::new(r"[^a-z0-9]+")
        .expect("static pattern")
        .replace_all(&value.to_ascii_lowercase(), "-")
        .trim_matches('-')
        .to_owned()
}

fn version_order(value: &str) -> Vec<u64> {
    Regex::new(r"\d+")
        .expect("static pattern")
        .find_iter(value)
        .filter_map(|part| part.as_str().parse().ok())
        .collect()
}

fn score_column(file: &str) -> Option<&'static str> {
    match file {
        "gpqa_diamond.csv" => Some("mean_score"),
        "hle_external.csv" => Some("Accuracy"),
        "gdp_pdf_external.csv" => Some("GDP.pdf score"),
        _ => None,
    }
}
