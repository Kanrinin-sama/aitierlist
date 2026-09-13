use crate::evidence::{
    BenchmarkObservation, BenchmarkSeries, ExecutionIdentity, ModelSubject, ScoreObservation,
    SourceRef, SourceSnapshot,
};
use anyhow::{Result, bail};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub fn parse(source: SourceRef, payload: Value) -> Result<SourceSnapshot> {
    if source.url.contains("agi.safe.ai")
        || source.source_id.to_ascii_lowercase().contains("rolling")
    {
        bail!("HLE-Rolling has no confirmed public result feed")
    }
    let mut owned = Vec::new();
    if let Some(html) = payload.as_str() {
        owned = html_rows(html);
    }
    let mut borrowed = Vec::new();
    collect_rows(&payload, &mut borrowed);
    let observations: Vec<_> = borrowed
        .into_iter()
        .chain(owned.iter())
        .filter_map(|row| observation(&source, row))
        .collect();
    if !payload.is_array() && !payload.is_object() && !payload.is_string() {
        bail!("unsupported HLE payload")
    }
    if observations.is_empty() {
        bail!("HLE payload has no usable observations")
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

fn collect_rows<'a>(value: &'a Value, rows: &mut Vec<&'a Map<String, Value>>) {
    match value {
        Value::Array(values) => values.iter().for_each(|value| collect_rows(value, rows)),
        Value::Object(object) => {
            if text(object, &["model", "modelName", "name"]).is_some()
                && number(object, &["accuracy", "score", "value"]).is_some()
            {
                rows.push(object);
            } else {
                object.values().for_each(|value| collect_rows(value, rows));
            }
        }
        _ => {}
    }
}

fn html_rows(html: &str) -> Vec<Map<String, Value>> {
    let row_pattern = Regex::new(r"(?s)<tr[^>]*>(.*?)</tr>").expect("static pattern");
    let cell_pattern = Regex::new(r"(?s)<t[dh][^>]*>(.*?)</t[dh]>").expect("static pattern");
    let tags = Regex::new(r"<[^>]+>").expect("static pattern");
    row_pattern
        .captures_iter(html)
        .filter_map(|row| {
            let cells: Vec<_> = cell_pattern
                .captures_iter(&row[1])
                .map(|cell| decode(tags.replace_all(&cell[1], " ").trim()))
                .collect();
            if cells.len() < 2 {
                return None;
            }
            let accuracy = Regex::new(r"-?\d+(?:\.\d+)?")
                .expect("static pattern")
                .find(&cells[1])?
                .as_str()
                .parse::<f64>()
                .ok()?;
            let mut result = Map::new();
            result.insert("model".to_owned(), Value::String(cells[0].clone()));
            result.insert("accuracy".to_owned(), Value::from(accuracy));
            if let Some(calibration) = cells.get(2) {
                result.insert(
                    "calibrationError".to_owned(),
                    Value::String(calibration.clone()),
                );
            }
            Some(result)
        })
        .collect()
}

fn observation(source: &SourceRef, row: &Map<String, Value>) -> Option<BenchmarkObservation> {
    let model = text(row, &["model", "modelName", "name"])?;
    let raw_model_id = text(row, &["modelId", "model_id", "id"]).unwrap_or(model);
    let score = number(row, &["accuracy", "score", "value"])?;
    let score = score / 100.0;
    if !score.is_finite() || !(0.0..=1.0).contains(&score) {
        return None;
    }
    let text_only = row
        .get("textOnly")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || model.to_ascii_lowercase().contains("text-only")
        || model.trim().ends_with('*');
    let tool_variant = text(row, &["variant", "protocol", "promptVariant"])
        .unwrap_or("")
        .to_ascii_lowercase();
    let tools = tool_variant.contains("tool") && !tool_variant.contains("no-tool");
    let variant = if text_only {
        if tools {
            "text-only-tools"
        } else {
            "text-only-no-tools"
        }
    } else if tools {
        "full-tools"
    } else {
        "full-no-tools"
    };
    let version = text(row, &["version", "datasetVersion"])
        .filter(|value| !value.is_empty())
        .unwrap_or("2025-04-03");
    let harness = text(row, &["harness", "evaluationHarness"]).unwrap_or("HLE official");
    let grader = text(row, &["grader", "judge", "judgeModel"]).unwrap_or("o3-mini");
    let protocol = text(row, &["protocol", "promptVariant"]).unwrap_or("official leaderboard");
    let extra: BTreeMap<_, _> = row
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let comparable_series_id = format!("hle:{variant}:{protocol}:{grader}");
    Some(BenchmarkObservation {
        observation_id: format!(
            "{}:hle:{raw_model_id}:{variant}:{comparable_series_id}",
            source.revision
        ),
        source: source.clone(),
        series: BenchmarkSeries {
            family: "hle".to_owned(),
            variant: variant.to_owned(),
            version: version.to_owned(),
            version_order: version_order(version),
            task_count: Some(if text_only { 2158 } else { 2500 }),
            comparable_series_id,
        },
        subject: ModelSubject {
            provider: text(row, &["provider", "modelProvider", "organization"])
                .map(str::to_owned)
                .unwrap_or_else(|| provider(model).to_owned()),
            model: model.trim().trim_end_matches('*').trim().to_owned(),
            raw_model_id: raw_model_id.to_owned(),
            effort: text(row, &["effort", "reasoningEffort", "reasoning_effort"])
                .map(str::to_owned),
        },
        execution: ExecutionIdentity {
            harness: harness.to_owned(),
            harness_version: text(row, &["harnessVersion"]).map(str::to_owned),
            protocol: protocol.to_owned(),
            grader: grader.to_owned(),
        },
        score: Some(ScoreObservation {
            metric: "accuracy".to_owned(),
            value: score,
            scale_min: Some(0.0),
            scale_max: Some(1.0),
            higher_is_better: true,
        }),
        resources: None,
        extra,
    })
}

fn text<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| object.get(*key)?.as_str())
}

fn number(object: &Map<String, Value>, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|key| object.get(*key)?.as_f64())
}

fn version_order(value: &str) -> Vec<u64> {
    Regex::new(r"\d+")
        .expect("static pattern")
        .find_iter(value)
        .filter_map(|part| part.as_str().parse().ok())
        .collect()
}

fn decode(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#x27;", "'")
        .replace("&quot;", "\"")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn provider(model: &str) -> &'static str {
    let model = model.to_ascii_lowercase();
    if model.contains("claude") || model.contains("opus") || model.contains("sonnet") {
        "anthropic"
    } else if model.contains("gpt") || model.contains("o1") || model.contains("o3") {
        "openai"
    } else if model.contains("gemini") {
        "google"
    } else if model.contains("grok") {
        "xai"
    } else if model.contains("deepseek") {
        "deepseek"
    } else if model.contains("qwen") {
        "alibaba"
    } else {
        "unknown"
    }
}
