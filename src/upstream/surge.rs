use anyhow::{Context, Result};
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::evidence::{
    BenchmarkObservation, BenchmarkSeries, ExecutionIdentity, ModelSubject, ScoreObservation,
    SourceRef, SourceSnapshot,
};

pub fn parse(source: SourceRef, payload: Value) -> Result<SourceSnapshot> {
    let html = payload
        .as_str()
        .or_else(|| payload["html"].as_str())
        .context("Surge payload has no HTML")?;
    let series = series(&source)?;
    let table_marker = Regex::new(r#"<[^>]+data-leaderboard-table(?:=|[ >])"#)?;
    let mut tables = table_marker.find_iter(html);
    let table_start = tables
        .next()
        .context("Surge leaderboard table is unavailable")?
        .start();
    let table_end = tables.next().map_or(html.len(), |next| next.start());
    let remaining = &html[table_start..];
    let table_end = table_end - table_start;
    let table = &remaining[..table_end];
    let rank_marker = Regex::new(r#"data-leaderboard-rank[^>]*>"#)?;
    let positions = rank_marker
        .find_iter(table)
        .map(|m| m.start())
        .collect::<Vec<_>>();
    anyhow::ensure!(
        !positions.is_empty(),
        "Surge leaderboard has no ranked rows"
    );
    let mut observations = Vec::with_capacity(positions.len());
    for (index, rank) in positions.iter().copied().enumerate() {
        let Some((start, end)) = enclosing_element(table, rank) else {
            continue;
        };
        if let Some(observation) = parse_row(&source, &series, &table[start..end], index + 1)? {
            observations.push(observation);
        }
    }
    anyhow::ensure!(
        !observations.is_empty(),
        "Surge leaderboard has no usable observations"
    );
    Ok(SourceSnapshot {
        source,
        payload: Some(payload),
        observations,
        fetch_error: None,
        last_good_revision: None,
        last_good_fetched_at: None,
    })
}

fn enclosing_element(html: &str, attribute: usize) -> Option<(usize, usize)> {
    let start = html[..attribute].rfind('<')?;
    let opening_end = html[start..].find('>')? + start + 1;
    let opening = &html[start + 1..opening_end - 1];
    let tag = opening
        .trim_start()
        .split(|character: char| {
            character.is_ascii_whitespace() || character == '>' || character == '/'
        })
        .next()?;
    if tag.is_empty() || opening.trim_end().ends_with('/') {
        return None;
    }
    let pattern = Regex::new(&format!(r"(?i)</?{}\b[^>]*>", regex::escape(tag))).ok()?;
    let mut depth = 0;
    for found in pattern.find_iter(&html[start..]) {
        let element = found.as_str();
        if element.as_bytes().get(1) == Some(&b'/') {
            depth -= 1;
            if depth == 0 {
                return Some((start, start + found.end()));
            }
        } else if !element.trim_end_matches('>').trim_end().ends_with('/') {
            depth += 1;
        }
    }
    None
}

#[derive(Clone)]
struct Series {
    family: String,
    variant: String,
    version: String,
    order: Vec<u64>,
    task_count: Option<usize>,
    comparable: String,
    protocol: String,
    grader: String,
}

fn series(source: &SourceRef) -> Result<Series> {
    let id = format!("{} {}", source.source_id, source.url).to_lowercase();
    let version = "unversioned".to_owned();
    let order = Vec::new();
    if id.contains("gdp-pdf") {
        return Ok(Series {
            family: "gdp-pdf".to_owned(),
            variant: "GDP.pdf".to_owned(),
            version,
            order,
            task_count: Some(100),
            comparable: "surge-gdp-pdf".to_owned(),
            protocol: "GDP.pdf five runs per task without tools".to_owned(),
            grader: "Gemini 3.5 Flash judge".to_owned(),
        });
    }
    if id.contains("chartography") {
        return Ok(generic("chartography", "Chartography", version, order));
    }
    if id.contains("hemingway") {
        return Ok(generic("writing", "Hemingway Bench", version, order));
    }
    if id.contains("ifeval") {
        return Ok(generic("instruction-following", "IFEval", version, order));
    }
    anyhow::bail!("Surge source does not identify a supported leaderboard")
}

fn generic(family: &str, variant: &str, version: String, order: Vec<u64>) -> Series {
    Series {
        family: family.to_owned(),
        variant: variant.to_owned(),
        version,
        order,
        task_count: None,
        comparable: format!("surge-{}", family.replace('_', "-")),
        protocol: format!("Surge {variant} published protocol"),
        grader: "Published benchmark grader".to_owned(),
    }
}

fn parse_row(
    source: &SourceRef,
    series: &Series,
    row: &str,
    fallback_rank: usize,
) -> Result<Option<BenchmarkObservation>> {
    let score = capture_attr(row, "data-score")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite());
    let Some(score) = score else { return Ok(None) };
    let brand = capture_class_text(row, "head-rank-table-brand").unwrap_or_default();
    let name = capture_class_text(row, "head-rank-table-name").unwrap_or_default();
    let raw_model = html_text(&format!("{brand} {name}"));
    if raw_model.is_empty() {
        return Ok(None);
    }
    let rank = capture_attr(row, "data-leaderboard-rank")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(fallback_rank);
    let (model, effort) = model_and_effort(&raw_model);
    let metric = capture_attr(row, "fs-list-field").unwrap_or("published-score");
    let mut extra = BTreeMap::new();
    extra.insert("published_rank".to_owned(), Value::from(rank));
    if let Some(lower) =
        capture_attr(row, "data-leaderboard-ci-lower").and_then(|v| v.parse::<f64>().ok())
    {
        extra.insert("confidence_interval_lower".to_owned(), Value::from(lower));
    }
    if let Some(upper) =
        capture_attr(row, "data-leaderboard-ci-upper").and_then(|v| v.parse::<f64>().ok())
    {
        extra.insert("confidence_interval_upper".to_owned(), Value::from(upper));
    }
    Ok(Some(BenchmarkObservation {
        observation_id: format!(
            "surge:{}:{}:{}:{}",
            series.comparable,
            source.revision,
            canonical(&model),
            effort.as_deref().unwrap_or("none")
        ),
        source: source.clone(),
        series: BenchmarkSeries {
            family: series.family.clone(),
            variant: series.variant.clone(),
            version: series.version.clone(),
            version_order: series.order.clone(),
            task_count: series.task_count,
            comparable_series_id: series.comparable.clone(),
        },
        subject: ModelSubject {
            provider: provider(&model).to_owned(),
            model,
            raw_model_id: raw_model,
            effort,
        },
        execution: ExecutionIdentity {
            harness: "Surge leaderboard".to_owned(),
            harness_version: None,
            protocol: series.protocol.clone(),
            grader: series.grader.clone(),
        },
        score: Some(ScoreObservation {
            metric: metric.to_owned(),
            value: score,
            scale_min: if metric.contains("elo") {
                None
            } else {
                Some(0.0)
            },
            scale_max: if metric.contains("elo") {
                None
            } else {
                Some(100.0)
            },
            higher_is_better: true,
        }),
        resources: None,
        extra,
    }))
}

fn capture_attr<'a>(html: &'a str, attribute: &str) -> Option<&'a str> {
    let needle = format!(r#"{attribute}="#);
    let start = html.find(&needle)? + needle.len();
    let quote = html.as_bytes().get(start).copied()? as char;
    if quote != '\'' && quote != '"' {
        return None;
    }
    let value = &html[start + 1..];
    Some(&value[..value.find(quote)?])
}

fn capture_class_text(html: &str, class: &str) -> Option<String> {
    let at = html.find(class)?;
    let tail = &html[at..];
    let open = tail.find('>')? + 1;
    let nested = &tail[open..];
    let value = if nested.trim_start().starts_with("<div") {
        let inner = nested.find('>')? + 1;
        &nested[inner..nested.find("</div>")?]
    } else {
        &nested[..nested.find("</div>")?]
    };
    Some(html_text(value))
}

fn html_text(value: &str) -> String {
    let tags = Regex::new(r"<[^>]*>").expect("static HTML tag expression");
    tags.replace_all(value, " ")
        .replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn provider(model: &str) -> &str {
    let lower = model.to_lowercase();
    if lower.contains("claude") {
        "anthropic"
    } else if lower.contains("gemini") {
        "google"
    } else if lower.contains("gpt") || lower.contains("o3") || lower.contains("o4") {
        "openai"
    } else if lower.contains("grok") {
        "xai"
    } else if lower.contains("fable") {
        "anthropic"
    } else if lower.contains("muse") {
        "muse"
    } else {
        ""
    }
}

fn model_and_effort(raw_model: &str) -> (String, Option<String>) {
    let Some(start) = raw_model.rfind('(') else {
        return (raw_model.to_owned(), None);
    };
    let Some(qualifier) = raw_model[start + 1..].strip_suffix(')').map(str::trim) else {
        return (raw_model.to_owned(), None);
    };
    let normalized = qualifier.to_ascii_lowercase();
    let effort = if normalized.contains("xhigh") || normalized.contains("x-high") {
        Some("xhigh")
    } else if normalized.contains("max") {
        Some("max")
    } else if normalized == "high" || normalized == "high reasoning" {
        Some("high")
    } else if normalized == "medium" || normalized == "medium reasoning" {
        Some("medium")
    } else if normalized == "low" || normalized == "low reasoning" {
        Some("low")
    } else {
        None
    };
    match effort {
        Some(effort) => (
            raw_model[..start].trim().to_owned(),
            Some(effort.to_owned()),
        ),
        None => (raw_model.to_owned(), None),
    }
}

fn canonical(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
