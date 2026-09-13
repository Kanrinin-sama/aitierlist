use crate::evidence::{
    BenchmarkObservation, BenchmarkSeries, ExecutionIdentity, ModelSubject, ScoreObservation,
    SourceRef, SourceSnapshot,
};
use anyhow::{Result, bail};
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

pub fn parse(source: SourceRef, payload: Value) -> Result<SourceSnapshot> {
    let family = family(&source, &payload)?;
    let mut borrowed = Vec::new();
    collect_rows(&payload, &mut borrowed);
    let owned = payload.as_str().map(html_rows).unwrap_or_default();
    let observations: Vec<_> = borrowed
        .into_iter()
        .chain(owned.iter())
        .filter_map(|row| observation(&source, &family, row))
        .collect();
    if observations.is_empty() {
        bail!("Scale leaderboard payload has no usable observations")
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

fn family(source: &SourceRef, payload: &Value) -> Result<String> {
    let text = format!("{} {}", source.source_id, source.url).to_ascii_lowercase();
    let declared = payload
        .get("benchmark")
        .or_else(|| payload.get("slug"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    let identity = format!("{text} {declared}");
    if identity.contains("sweatlas-qna") || identity.contains("swe-atlas-qna") {
        Ok("qna".to_owned())
    } else if identity.contains("sweatlas-tw") || identity.contains("test-writing") {
        Ok("swe-atlas-test-writing".to_owned())
    } else if identity.contains("sweatlas-refactoring") || identity.contains("refactoring") {
        Ok("swe-atlas-refactoring".to_owned())
    } else if identity.contains("mcp_atlas") || identity.contains("mcp-atlas") {
        Ok("mcp-atlas".to_owned())
    } else {
        bail!("unsupported Scale leaderboard source")
    }
}

fn collect_rows<'a>(value: &'a Value, rows: &mut Vec<&'a Map<String, Value>>) {
    match value {
        Value::Array(values) => values.iter().for_each(|value| collect_rows(value, rows)),
        Value::Object(object) => {
            let model = text(object, &["model", "modelName", "name", "subject"]);
            let score = number(object, &["score", "passRate", "accuracy", "value"]);
            if model.is_some() && score.is_some() {
                rows.push(object);
            } else {
                object.values().for_each(|value| collect_rows(value, rows));
            }
        }
        _ => {}
    }
}

fn html_rows(html: &str) -> Vec<Map<String, Value>> {
    let chunks =
        Regex::new(r#"self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)"#).expect("static pattern");
    let decoded = chunks
        .captures_iter(html)
        .filter_map(|capture| serde_json::from_str::<String>(&format!("\"{}\"", &capture[1])).ok())
        .collect::<String>();
    let decoded = if decoded.is_empty() { html } else { &decoded };
    let mut rows = Vec::new();
    let mut offset = 0;
    while let Some(found) = decoded[offset..].find(r#""entries":["#) {
        let start = offset + found + r#""entries":"#.len();
        let Some(end) = array_end(decoded, start) else {
            break;
        };
        if let Ok(values) = serde_json::from_str::<Vec<Map<String, Value>>>(&decoded[start..end]) {
            rows.extend(values);
        }
        offset = end;
    }
    rows
}

fn array_end(text: &str, start: usize) -> Option<usize> {
    let mut depth = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (offset, character) in text[start..].char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else if character == '"' {
            quoted = true;
        } else if character == '[' {
            depth += 1;
        } else if character == ']' {
            depth -= 1;
            if depth == 0 {
                return Some(start + offset + character.len_utf8());
            }
        }
    }
    None
}

fn observation(
    source: &SourceRef,
    family: &str,
    row: &Map<String, Value>,
) -> Option<BenchmarkObservation> {
    let raw_model = text(row, &["model", "modelName", "name", "subject"])?.trim();
    let raw_model_id = text(row, &["modelId", "model_id", "id"]).unwrap_or(raw_model);
    let score = number(row, &["score", "passRate", "accuracy", "value"])?;
    if !score.is_finite() {
        return None;
    }
    let normalized = score / 100.0;
    if !(0.0..=1.0).contains(&normalized) {
        return None;
    }
    let variant = text(row, &["variant", "protocol", "subset"]).unwrap_or("official-leaderboard");
    let version = text(row, &["version", "datasetVersion"])
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    let embedded_harness = Regex::new(r"\(([^()]*(?:Code|Codex|CLI|Agent)[^()]*)\)")
        .expect("static pattern")
        .captures(raw_model)
        .map(|capture| capture[1].trim().to_owned());
    let harness = text(row, &["harness", "agent", "scaffold"])
        .or(embedded_harness.as_deref())
        .unwrap_or("unspecified");
    let effort = text(row, &["effort", "reasoningEffort", "reasoning_effort"])
        .map(str::to_owned)
        .or_else(|| {
            Regex::new(r"(?i)\b(xhigh|x-high|max|high|medium|low|none)\*?$")
                .expect("static pattern")
                .captures(raw_model)
                .map(|capture| capture[1].to_ascii_lowercase().replace('-', ""))
        });
    let without_harness = Regex::new(r"\s*\([^()]+\)\s*")
        .expect("static pattern")
        .replace_all(raw_model, " ")
        .trim()
        .to_owned();
    let model = Regex::new(r"(?i:\s+(?:xhigh|x-high|max|high|medium|low|none)\*?$)")
        .expect("static pattern")
        .replace(&without_harness, "")
        .trim_end_matches('*')
        .trim()
        .to_owned();
    let task_count = number(row, &["taskCount", "tasks"])
        .map(|value| value as usize)
        .or(match family {
            "qna" => Some(124),
            "swe-atlas-test-writing" => Some(90),
            "swe-atlas-refactoring" => Some(70),
            "mcp-atlas" => Some(1000),
            _ => None,
        });
    let protocol = text(row, &["protocol", "evaluationProtocol"]).unwrap_or(match family {
        "qna" => "task-resolve-strict-rubric",
        "swe-atlas-test-writing" => "tests-and-software-quality-rubric",
        "swe-atlas-refactoring" => "gold-tests-and-must-have-rubrics",
        "mcp-atlas" => "coverage-at-least-0.75",
        _ => variant,
    });
    let grader = text(row, &["grader", "judge", "judgeModel"]).unwrap_or(match family {
        "qna" | "swe-atlas-test-writing" | "swe-atlas-refactoring" => "Claude Opus 4.5",
        _ => "unspecified",
    });
    let extra: BTreeMap<_, _> = row
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let comparable_series_id = if family == "qna" {
        "swe-atlas-qna".to_owned()
    } else {
        format!("scale:{family}:{variant}:{protocol}:{grader}")
    };
    let mut observation_source = source.clone();
    if let Some(created_at) = text(row, &["createdAt", "publishedAt"]) {
        observation_source.published_at = Some(created_at.to_owned());
    }
    Some(BenchmarkObservation {
        observation_id: format!(
            "{}:{family}:{raw_model_id}:{effort:?}:{comparable_series_id}",
            source.revision
        ),
        source: observation_source,
        series: BenchmarkSeries {
            family: family.to_owned(),
            variant: variant.to_owned(),
            version: version.to_owned(),
            version_order: version_order(version),
            task_count,
            comparable_series_id,
        },
        subject: ModelSubject {
            provider: text(
                row,
                &["provider", "modelProvider", "organization", "company"],
            )
            .map(str::to_owned)
            .unwrap_or_else(|| provider(&model).to_owned()),
            model,
            raw_model_id: raw_model_id.to_owned(),
            effort,
        },
        execution: ExecutionIdentity {
            harness: harness.to_owned(),
            harness_version: text(row, &["harnessVersion", "agentVersion"]).map(str::to_owned),
            protocol: protocol.to_owned(),
            grader: grader.to_owned(),
        },
        score: Some(ScoreObservation {
            metric: if family == "qna" {
                "accuracy".to_owned()
            } else {
                text(row, &["metric"]).unwrap_or("pass_rate").to_owned()
            },
            value: normalized,
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

fn provider(model: &str) -> &'static str {
    let model = model.to_ascii_lowercase();
    if model.contains("claude")
        || model.contains("opus")
        || model.contains("sonnet")
        || model.contains("fable")
    {
        "anthropic"
    } else if model.contains("gpt") || model.contains("codex") || model.contains("o3") {
        "openai"
    } else if model.contains("gemini") {
        "google"
    } else if model.contains("grok") {
        "xai"
    } else if model.contains("glm") {
        "zai"
    } else if model.contains("kimi") {
        "moonshot"
    } else if model.contains("minimax") {
        "minimax"
    } else if model.contains("muse") {
        "muse"
    } else {
        "unknown"
    }
}
