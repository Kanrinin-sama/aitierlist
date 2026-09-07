use crate::cache;
use crate::types::{CacheState, Row};
use aes_gcm::{
    Aes128Gcm, Aes256Gcm, Nonce,
    aead::{Aead, KeyInit},
};
use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use regex::Regex;
use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::Read};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SWE_TASKS: f64 = 113.0;
const TERMINAL_TASKS: f64 = 89.0;
const QNA_TASKS: f64 = 124.0;
const GPQA_TASKS: f64 = 198.0;
const HLE_TASKS: f64 = 2158.0;
const LCR_TASKS: f64 = 100.0;
pub const EXCLUDED_EFFORTS: [&str; 1] = ["none"];
const SITE: &str = "https://artificialanalysis.ai";

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
}
fn finite_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|value| value.is_finite())
}
fn label(value: &Value) -> &str {
    value.as_str().unwrap_or("")
}
fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn regex(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static regular expression")
}
fn present(value: &Value) -> bool {
    !value.is_null()
}
fn fallback(value: &Value, default: f64) -> f64 {
    value.as_f64().unwrap_or(default)
}

pub fn median(values: impl IntoIterator<Item = f64>) -> Option<f64> {
    let mut values: Vec<_> = values.into_iter().filter(|value| !value.is_nan()).collect();
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.is_empty() {
        None
    } else if values.len().is_multiple_of(2) {
        Some((values[middle - 1] + values[middle]) / 2.0)
    } else {
        Some(values[middle])
    }
}

pub fn harness_key(name: &str) -> String {
    let name = display_name(name).to_lowercase();
    let name = regex(r"^claude\s+").replace(&name, "");
    regex(r"\s+").replace_all(&name, " ").trim().to_owned()
}

pub fn family_key(harness: &str, model_key: &str) -> String {
    format!(
        "{harness}|{}",
        regex(r"\s*\((?:max|xhigh|high|medium|low|minimal|none)\)$").replace(model_key, "")
    )
}

fn effort_of(model_key: &str) -> Option<String> {
    regex(r"\((max|xhigh|high|medium|low|minimal|none)\)$")
        .captures(model_key)
        .map(|capture| capture[1].to_owned())
}

pub fn display_name(raw_name: &str) -> String {
    let groups: Vec<_> = regex(r"\(([^)]*)\)")
        .captures_iter(raw_name)
        .map(|capture| capture[1].to_lowercase())
        .collect();
    let effort = groups.iter().rev().find_map(|qualifiers| {
        [
            (r"\b(?:non[- ]reasoning|none|no reasoning)\b", "none"),
            (r"\b(?:max|maximum)\b", "max"),
            (r"\b(?:xhigh|extra[- ]high)\b", "xhigh"),
            (r"\bhigh\b", "high"),
            (r"\b(?:medium|med)\b", "medium"),
            (r"\blow\b", "low"),
            (r"\b(?:minimal|minimum|min)\b", "minimal"),
        ]
        .into_iter()
        .find_map(|(pattern, effort)| regex(pattern).is_match(qualifiers).then_some(effort))
    });
    let base = regex(r"\s*\([^)]*\)")
        .replace_all(raw_name, "")
        .trim()
        .to_owned();
    effort.map_or_else(|| base.clone(), |effort| format!("{base} ({effort})"))
}

fn clean_label(name: &str) -> String {
    let name = regex(r"(?i)\s*with fallback").replace(name, "");
    regex(r"\s*\(\s*\)").replace(&name, "").trim().to_owned()
}

pub fn vendor_key(harness: &str, model: &str) -> Option<&'static str> {
    for (pattern, vendor) in [
        ("antigravity|gemini cli", "google"),
        ("grok build", "xai"),
        ("muse code", "muse"),
        ("kimi code cli", "moonshot"),
        ("devin cli", "cognition"),
    ] {
        if regex(&format!("(?i){pattern}")).is_match(harness) {
            return Some(vendor);
        }
    }
    if regex("(?i)cursor cli").is_match(harness) {
        return Some(if regex("(?i)^composer").is_match(model) {
            "cursor"
        } else {
            "cursor-api"
        });
    }
    if regex("(?i)opencode").is_match(harness)
        && regex(r"(?i)\b(?:qwen|deepseek|glm|kimi|muse contributor)").is_match(model)
    {
        return Some("opencode");
    }
    for (pattern, vendor) in [
        ("deepseek", "deepseek"),
        ("qwen", "alibaba"),
        ("glm", "zai"),
        ("kimi", "moonshot"),
        ("minimax", "minimax"),
        ("muse", "muse"),
        ("composer", "cursor"),
        ("grok", "xai"),
        (r"(?:gpt|codex|o\d+\b)", "openai"),
        ("(?:claude|fable|opus|sonnet)", "anthropic"),
        ("(?:gemini|antigravity)", "google"),
    ] {
        if regex(&format!(r"(?i)\b{pattern}")).is_match(model) {
            return Some(vendor);
        }
    }
    None
}

struct ResponseBody {
    bytes: Vec<u8>,
    etag: Option<String>,
}

struct ModelMatch<'a> {
    item: &'a Value,
    key: String,
    effort: Option<String>,
    family: String,
}

fn preferred_model<'a>(items: Vec<&ModelMatch<'a>>) -> Option<&'a Value> {
    let current: Vec<_> = items
        .iter()
        .copied()
        .filter(|model| model.item["deprecated"] != true)
        .collect();
    let candidates = if current.is_empty() { &items } else { &current };
    let configurations: std::collections::BTreeSet<_> =
        candidates.iter().map(|model| &model.key).collect();
    (configurations.len() == 1).then(|| candidates[0].item)
}

async fn fetch_response(
    client: &Client,
    path: &str,
    etag: Option<&str>,
) -> Result<Option<ResponseBody>> {
    let url = format!("{SITE}{path}");
    let mut request = client.get(&url);
    if let Some(etag) = etag {
        request = request.header(reqwest::header::IF_NONE_MATCH, etag);
    }
    let response = request
        .send()
        .await
        .with_context(|| format!("fetching {url}"))?;
    if response.status() == StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    let mut response = response.error_for_status()?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > 64 * 1024 * 1024 - bytes.len() {
            bail!("AA response exceeds 64 MiB: {url}");
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(Some(ResponseBody { bytes, etag }))
}

fn object_end(text: &str, start: usize) -> Result<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut in_string = false;
    let mut index = start;
    while index < bytes.len() {
        let character = bytes[index];
        if in_string {
            if character == b'\\' {
                index += 1;
            } else if character == b'"' {
                in_string = false;
            }
        } else if character == b'"' {
            in_string = true;
        } else if character == b'{' {
            depth += 1;
        } else if character == b'}' {
            depth -= 1;
            if depth == 0 {
                return Ok(index + 1);
            }
        }
        index += 1;
    }
    bail!("Unterminated object in agents payload")
}

pub fn parse_agent_rows(html: &str) -> Result<Value> {
    let mut payload = String::new();
    for capture in regex(r#"self\.__next_f\.push\(\[1,"((?:[^"\\]|\\.)*)"\]\)"#).captures_iter(html)
    {
        payload.push_str(&serde_json::from_str::<String>(&format!(
            "\"{}\"",
            &capture[1]
        ))?);
    }
    let mut rows = Vec::<Value>::new();
    for found in regex(r#"\{"id":"[0-9a-f]{32}","isDefault""#).find_iter(&payload) {
        let row: Value =
            serde_json::from_str(&payload[found.start()..object_end(&payload, found.start())?])?;
        if let Some(existing) = rows.iter_mut().find(|existing| existing["id"] == row["id"]) {
            *existing = row;
        } else {
            rows.push(row);
        }
    }
    if rows.is_empty() {
        bail!("No agent rows on /agents/coding-agents");
    }
    Ok(json!(rows))
}

async fn fetch_agent_rows(client: &Client, cached: &Value) -> Result<(Value, Value)> {
    let path = "/agents/coding-agents";
    let metadata = &cached["http"]["agents"];
    match fetch_response(client, path, metadata["etag"].as_str()).await? {
        Some(response) => Ok((
            parse_agent_rows(std::str::from_utf8(&response.bytes)?)?,
            json!({"etag":response.etag}),
        )),
        None if cached["agents"].is_array() => Ok((cached["agents"].clone(), metadata.clone())),
        None => bail!("304 without cached agents"),
    }
}

async fn fetch_manifest_payload(
    client: &Client,
    page: &str,
    cached: &Value,
    field: &str,
) -> Result<(Value, Value)> {
    let metadata = &cached["http"][field];
    let response = fetch_response(client, page, metadata["etag"].as_str()).await?;
    let Some(response) = response else {
        if present(&cached[field]) {
            return Ok((cached[field].clone(), metadata.clone()));
        }
        bail!("304 without cached {field}");
    };
    let html = std::str::from_utf8(&response.bytes)?;
    let pattern = regex(r#"manifest\\":\{\\"path\\":\\"([^"]+)\\",\\"key\\":\\"([^"]+)\\"\}"#);
    let capture = pattern
        .captures(html)
        .with_context(|| format!("No manifest on {page}"))?;
    let path = &capture[1];
    let blob_etag = (metadata["path"].as_str() == Some(path)
        && metadata["key"].as_str() == Some(&capture[2]))
    .then(|| metadata["blobEtag"].as_str())
    .flatten();
    let encrypted = fetch_response(client, path, blob_etag).await?;
    let Some(encrypted) = encrypted else {
        if !present(&cached[field]) {
            bail!("304 without cached {field}");
        }
        let mut metadata = metadata.clone();
        metadata["etag"] = json!(response.etag);
        return Ok((cached[field].clone(), metadata));
    };
    let key = hex::decode(&capture[2]).context("decoding manifest key")?;
    let digest = Sha256::digest(&key);
    let nonce = Nonce::try_from(&digest[..12]).map_err(|_| anyhow!("invalid AES nonce"))?;
    let decrypted = match key.len() {
        16 => Aes128Gcm::new_from_slice(&key)
            .map_err(|_| anyhow!("invalid AES key"))?
            .decrypt(&nonce, encrypted.bytes.as_slice()),
        32 => Aes256Gcm::new_from_slice(&key)
            .map_err(|_| anyhow!("invalid AES key"))?
            .decrypt(&nonce, encrypted.bytes.as_slice()),
        _ => bail!("Unsupported AES key length"),
    }
    .map_err(|_| anyhow!("AES-GCM authentication failed for {page}"))?;
    let mut decoded = Vec::new();
    GzDecoder::new(decrypted.as_slice()).read_to_end(&mut decoded)?;
    Ok((
        serde_json::from_slice(&decoded)?,
        json!({"etag":response.etag,"path":path,"key":&capture[2],"blobEtag":encrypted.etag}),
    ))
}

pub fn fetch_payloads(cached: &Value) -> Result<Value> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async {
        let client = Client::builder().user_agent("Mozilla/5.0").timeout(std::time::Duration::from_secs(90)).build()?;
        let (agents, evaluation, catalog) = tokio::try_join!(fetch_agent_rows(&client, cached), fetch_manifest_payload(&client, "/evaluations/livecodebench", cached, "evaluation"), fetch_manifest_payload(&client, "/models", cached, "catalog"))?;
        Ok(json!({"fetchedAt":OffsetDateTime::now_utc().format(&Rfc3339)?,"agents":agents.0,"evaluation":evaluation.0,"catalog":catalog.0,"http":{"agents":agents.1,"evaluation":evaluation.1,"catalog":catalog.1}}))
    })
}

fn model_prices(item: &Value, hosts: &[&Value]) -> Option<(f64, f64, f64, f64)> {
    let price = |key: &str| {
        item[key]
            .as_f64()
            .or_else(|| median(hosts.iter().map(|host| number(&host[key]))))
            .filter(|price| price.is_finite() && *price >= 0.0)
    };
    let input = price("price1mInputTokens")?;
    let output = price("price1mOutputTokens")?;
    Some((
        input,
        output,
        price("cacheHitPrice").unwrap_or(input),
        price("cacheWritePrice").unwrap_or(input),
    ))
}

fn model_speed(item: &Value, hosts: &[&Value]) -> Option<f64> {
    let first_party: Vec<_> = hosts
        .iter()
        .copied()
        .filter(|host| host["host"]["name"] == item["creator"]["name"])
        .collect();
    median(
        if first_party.is_empty() {
            hosts
        } else {
            &first_party
        }
        .iter()
        .map(|host| number(&host["timescaleData"]["medianOutputSpeed"])),
    )
    .or_else(|| item["medianCanonicalAnswerOutputSpeed"].as_f64())
    .filter(|speed| speed.is_finite() && *speed > 0.0)
}

fn task_metric(
    benchmark: &str,
    pass: f64,
    seconds: f64,
    usd: f64,
    time_basis: &str,
    cost_basis: &str,
) -> Value {
    json!({
        "benchmark": benchmark,
        "pass": pass,
        "seconds": seconds,
        "pooledSeconds": seconds,
        "usd": usd,
        "timeBasis": time_basis,
        "costBasis": cost_basis
    })
}

fn canonical_resources(
    item: &Value,
    key: &str,
    tasks: f64,
    prices: Option<(f64, f64, f64, f64)>,
    speed: Option<f64>,
) -> Option<(Option<f64>, Option<f64>)> {
    let finite_token = |value: &Value| finite_number(value).filter(|value| *value >= 0.0);
    let tokens = &item["canonicalEvalTokenCounts"][key];
    let input = finite_token(&tokens["input"])?;
    let answer = finite_token(&tokens["answer"])?;
    let reasoning = finite_token(&tokens["reasoning"])?;
    let cacheable = fallback(&tokens["cacheableInput"], 0.0).clamp(0.0, input);
    let cached = if item["cacheHitRate"].is_number() {
        cacheable * number(&item["cacheHitRate"]).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let usd = prices.map(|(input_price, output_price, cache_price, write_price)| {
        let cache_write = cacheable - cached;
        ((input - cacheable) * input_price
            + cached * cache_price
            + cache_write * write_price
            + (answer + reasoning) * output_price)
            / 1e6
            / tasks
    });
    let seconds = speed.map(|speed| (answer + reasoning) / tasks / speed);
    Some((seconds, usd))
}

pub fn agent_row(raw: &Value, item: Option<&Value>, hosts: &[&Value]) -> Option<Value> {
    if raw["isUnavailable"].as_bool().unwrap_or(false)
        || EXCLUDED_EFFORTS.iter().any(|effort| {
            label(&raw["displayLabel"])
                .to_lowercase()
                .contains(&format!("({effort})"))
        })
    {
        return None;
    }
    let evaluations: serde_json::Map<String, Value> = array(&raw["evals"])
        .iter()
        .map(|evaluation| {
            (
                label(&evaluation["datasetIndexName"]).to_owned(),
                evaluation["mean"].clone(),
            )
        })
        .collect();
    let evaluations = Value::Object(evaluations);
    let swe = &evaluations["deep-swe"];
    let term = &evaluations["terminal-bench-v2.1"];
    let qna = &evaluations["swe-atlas-qna"];
    let mean = &raw["mean"];
    if [swe, term, qna, &mean["costUsd"], &mean["agentWallTimeSec"]]
        .iter()
        .any(|value| value.is_null())
    {
        return None;
    }
    if [swe, term, qna].iter().any(|evaluation| {
        ["inputTokens", "outputTokens"]
            .iter()
            .any(|key| finite_number(&evaluation[key]).is_none_or(|value| value < 0.0))
            || evaluation["cacheWriteTokens"]
                .as_f64()
                .is_some_and(|value| !value.is_finite() || value < 0.0)
    }) || [
        "costUsd",
        "agentWallTimeSec",
        "steps",
        "inputTokens",
        "outputTokens",
        "cacheTokens",
    ]
    .iter()
    .any(|key| finite_number(&mean[key]).is_none_or(|value| value < 0.0))
        || number(&mean["agentWallTimeSec"]) <= 0.0
        || number(&mean["inputTokens"]) <= 0.0
        || number(&mean["outputTokens"]) <= 0.0
        || number(&mean["steps"]) <= 0.0
    {
        return None;
    }
    let implementation_mean = |key: &str| {
        (number(&swe[key]) * SWE_TASKS + number(&term[key]) * TERMINAL_TASKS)
            / (SWE_TASKS + TERMINAL_TASKS)
    };
    let pass = finite_number(&swe["reward"])
        .zip(finite_number(&term["reward"]))
        .filter(|(swe, term)| (0.0..=1.0).contains(swe) && (0.0..=1.0).contains(term))
        .map(|_| implementation_mean("reward"));
    let pooled_seconds = number(&mean["agentWallTimeSec"]);
    let pooled_usd = number(&mean["costUsd"]);
    let weighted_mean_output = (number(&swe["outputTokens"]) * SWE_TASKS
        + number(&term["outputTokens"]) * TERMINAL_TASKS
        + number(&qna["outputTokens"]) * QNA_TASKS)
        / (SWE_TASKS + TERMINAL_TASKS + QNA_TASKS);
    let seconds_for = |evaluation: &Value| {
        if weighted_mean_output > 0.0 {
            pooled_seconds * number(&evaluation["outputTokens"]) / weighted_mean_output
        } else {
            pooled_seconds
        }
    };
    let prices = item.and_then(|item| model_prices(item, hosts));
    let cache_fraction =
        (number(&mean["cacheTokens"]) / number(&mean["inputTokens"])).clamp(0.0, 1.0);
    let write_fraction =
        (fallback(&mean["cacheWriteTokens"], 0.0) / number(&mean["inputTokens"])).clamp(0.0, 1.0);
    let priced = prices.is_some();
    let token_cost = |evaluation: &Value| {
        let (input_price, output_price, cache_price, write_price) = prices?;
        let input = number(&evaluation["inputTokens"]);
        let write = evaluation["cacheWriteTokens"]
            .as_f64()
            .unwrap_or(input * write_fraction)
            .clamp(0.0, input);
        let cached = (input * cache_fraction).clamp(0.0, input - write);
        Some(
            ((input - cached - write) * input_price
                + cached * cache_price
                + write * write_price
                + number(&evaluation["outputTokens"]) * output_price)
                / 1e6,
        )
    };
    let direct_unanchored = [(swe, SWE_TASKS), (term, TERMINAL_TASKS), (qna, QNA_TASKS)]
        .into_iter()
        .map(|(evaluation, tasks)| token_cost(evaluation).map(|cost| cost * tasks))
        .collect::<Option<Vec<_>>>()
        .map(|costs| costs.into_iter().sum::<f64>() / (SWE_TASKS + TERMINAL_TASKS + QNA_TASKS));
    let anchor = direct_unanchored
        .filter(|cost| *cost > 0.0)
        .map(|cost| pooled_usd / cost);
    let cost_for = |evaluation: &Value| {
        token_cost(evaluation)
            .zip(anchor)
            .map(|(cost, anchor)| cost * anchor)
            .unwrap_or(pooled_usd)
    };
    let time_basis = "Pooled observed wall time allocated in proportion to task output tokens";
    let cost_basis = if priced && anchor.is_some() {
        "Token-price estimate anchored to pooled observed cost; cache mix transferred"
    } else {
        "Pooled cost; token prices unavailable"
    };
    let swe_seconds = seconds_for(swe);
    let term_seconds = seconds_for(term);
    let qna_seconds = seconds_for(qna);
    let swe_usd = cost_for(swe);
    let term_usd = cost_for(term);
    let qna_usd = cost_for(qna);
    let wait =
        (swe_seconds * SWE_TASKS + term_seconds * TERMINAL_TASKS) / (SWE_TASKS + TERMINAL_TASKS);
    let attempt_usd =
        (swe_usd * SWE_TASKS + term_usd * TERMINAL_TASKS) / (SWE_TASKS + TERMINAL_TASKS);
    let mut metrics = vec![
        task_metric(
            "swe",
            number(&swe["reward"]),
            swe_seconds,
            swe_usd,
            time_basis,
            cost_basis,
        ),
        task_metric(
            "terminal",
            number(&term["reward"]),
            term_seconds,
            term_usd,
            time_basis,
            cost_basis,
        ),
        task_metric(
            "qna",
            number(&qna["reward"]),
            qna_seconds,
            qna_usd,
            time_basis,
            cost_basis,
        ),
    ];
    metrics.retain(|metric| {
        finite_number(&metric["pass"]).is_some_and(|value| (0.0..=1.0).contains(&value))
    });
    for metric in &mut metrics {
        metric["pooledSeconds"] = json!(pooled_seconds);
    }
    let model_key = harness_key(label(&raw["display"]["model"]));
    let harness = label(&raw["agentName"]);
    let mut row = json!({
        "name":clean_label(label(&raw["displayLabel"])),"rawName":raw["displayLabel"],"model":raw["display"]["model"],"harness":harness,"modelKey":model_key,"family":family_key(harness,&model_key),"vendor":vendor_key(harness,&model_key),
        "swe":swe["reward"],"term":term["reward"],"qna":qna["reward"],"pass":pass,
        "waitSeconds":wait,"attemptUsd":attempt_usd,"readSeconds":qna_seconds,"readUsd":qna_usd,"pooledSeconds":pooled_seconds,"usdPerStep":number(&mean["costUsd"]) / number(&mean["steps"]),"taskMetrics":metrics
    });
    if let Some(item) = item {
        apply_model_quality(&mut row, item);
    }
    Some(row)
}

pub fn model_row(item: &Value, hosts: &[&Value]) -> Value {
    let prices = model_prices(item, hosts);
    let speed = model_speed(item, hosts);
    let rankable = !item["deprecated"].as_bool().unwrap_or(false)
        && fallback(&item["terminalbenchV21"], 0.0) > 0.0
        && !item["canonicalEvalTokenCounts"]["terminalbenchV21"].is_null()
        && item["gpqa"].is_number()
        && item["hle"].is_number()
        && prices.is_some()
        && speed.is_some_and(|speed| speed != 0.0 && !speed.is_nan());
    let name = display_name(label(&item["name"]));
    let key = harness_key(label(&item["name"]));
    let mut metrics = Vec::new();
    if let Some(prices) = prices {
        let mut terminal_proxy = None;
        for (benchmark, score_key, token_key, tasks) in [
            (
                "terminal",
                "terminalbenchV21",
                "terminalbenchV21",
                TERMINAL_TASKS,
            ),
            ("gpqa", "gpqa", "gpqa", GPQA_TASKS),
            ("hle", "hle", "hle", HLE_TASKS),
            ("lcr", "lcr", "lcr", LCR_TASKS),
        ] {
            let Some(pass) = item[score_key].as_f64() else {
                continue;
            };
            if let Some((Some(seconds), Some(usd))) =
                canonical_resources(item, token_key, tasks, Some(prices), speed)
            {
                if benchmark == "terminal" {
                    terminal_proxy = Some((seconds, usd));
                }
                metrics.push(task_metric(
                    benchmark,
                    pass,
                    seconds,
                    usd,
                    "Canonical output decode estimate per benchmark task",
                    "Canonical token-price estimate per benchmark task; cache misses priced as writes",
                ));
            } else if let Some((seconds, usd)) = terminal_proxy {
                metrics.push(task_metric(
                    benchmark,
                    pass,
                    seconds,
                    usd,
                    "Terminal resource proxy; canonical tokens unavailable",
                    "Terminal resource proxy; canonical tokens unavailable",
                ));
            }
        }
    }
    let terminal_metric = metrics
        .iter()
        .find(|metric| metric["benchmark"] == "terminal");
    let terminal_seconds = terminal_metric
        .and_then(|metric| metric["seconds"].as_f64())
        .unwrap_or(0.0);
    let terminal_usd = terminal_metric
        .and_then(|metric| metric["usd"].as_f64())
        .unwrap_or(0.0);
    let mut row = json!({"name":name,"rawName":item["name"],"model":name,"modelKey":key,"harness":"model","family":family_key("model",&key),"vendor":vendor_key("model",&key),"term":item["terminalbenchV21"],"gpqa":item["gpqa"],"hle":item["hle"],"logic":if item["gpqa"].is_number() && item["hle"].is_number() { Some((number(&item["gpqa"])+number(&item["hle"]))/2.0) } else { None },"pass":item["terminalbenchV21"],"rankable":rankable,"speed":speed,"waitSeconds":terminal_seconds,"attemptUsd":terminal_usd,"readSeconds":terminal_seconds,"readUsd":terminal_usd,"taskMetrics":metrics});
    row["deprecated"] = json!(item["deprecated"].as_bool().unwrap_or(false));
    apply_model_quality(&mut row, item);
    row
}

pub fn apply_model_quality(row: &mut Value, item: &Value) {
    row["index"] = json!(
        item["intelligenceIndex"]
            .as_f64()
            .map(|value| value / 100.0)
    );
    row["gpqa"] = item["gpqa"].clone();
    row["hle"] = item["hle"].clone();
    row["lcr"] = item["lcr"].clone();
    row["logic"] = json!(
        item["gpqa"]
            .as_f64()
            .zip(item["hle"].as_f64())
            .map(|(gpqa, hle)| (gpqa + hle) / 2.0)
    );
    row["hallucination"] = item["omniscienceBreakdown"]["hallucinationRate"].clone();
}

fn disambiguate(rows: &mut [Value]) {
    let mut counts = BTreeMap::new();
    for row in rows.iter() {
        *counts.entry(label(&row["name"]).to_owned()).or_insert(0) += 1;
    }
    for row in rows {
        if counts[label(&row["name"])] > 1 {
            row["name"] = row["rawName"].clone();
        }
    }
}

pub fn rows_from_payloads(payloads: &Value) -> Result<Vec<Row>> {
    let agents = payloads["agents"]
        .as_array()
        .context("missing agents array")?;
    let evaluation = payloads["evaluation"]["models"]
        .as_array()
        .context("missing evaluation models")?;
    let catalog = payloads["catalog"]
        .as_array()
        .context("missing model catalog")?;
    let model_matches: Vec<_> = evaluation
        .iter()
        .map(|item| {
            let key = harness_key(label(&item["name"]));
            ModelMatch {
                effort: effort_of(&key),
                family: family_key("", &key),
                key,
                item,
            }
        })
        .collect();
    let mut agents: Vec<_> = agents
        .iter()
        .filter_map(|raw| {
            let slug = label(&raw["hostModelSlug"])
                .split_once('_')
                .map(|(_, slug)| slug)
                .unwrap_or("");
            let model_key = harness_key(label(&raw["display"]["model"]));
            let exact = model_matches
                .iter()
                .filter(|model| model.key == model_key)
                .collect();
            let effort = effort_of(&model_key);
            let compatible_slug = model_matches
                .iter()
                .filter(|model| label(&model.item["slug"]) == slug)
                .filter(|model| model.effort.as_deref() == effort.as_deref())
                .collect();
            let family = if effort.is_none() {
                let family = family_key("", &model_key);
                let candidates: Vec<_> = model_matches
                    .iter()
                    .filter(|model| model.family == family && model.item["deprecated"] != true)
                    .collect();
                preferred_model(candidates)
            } else {
                None
            };
            let item = preferred_model(exact)
                .or_else(|| preferred_model(compatible_slug))
                .or(family);
            let hosts: Vec<_> = item
                .map(|item| {
                    catalog
                        .iter()
                        .filter(|host| host["modelSlug"] == item["slug"])
                        .collect()
                })
                .unwrap_or_default();
            agent_row(raw, item, &hosts)
        })
        .collect();
    let mut models: Vec<_> = evaluation
        .iter()
        .map(|item| {
            model_row(
                item,
                &catalog
                    .iter()
                    .filter(|host| host["modelSlug"] == item["slug"])
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    disambiguate(&mut agents);
    disambiguate(&mut models);
    models.retain(|row| row["rankable"] == true);
    agents.extend(models);
    agents
        .into_iter()
        .map(|mut row| {
            row["effort"] = json!(effort_of(label(&row["modelKey"])));
            row["model"] = json!(
                regex(r"\s*\((?:max|xhigh|high|medium|low|minimal|none)\)$")
                    .replace(label(&row["model"]), "")
                    .to_string()
            );
            row["vendor"] = json!(label(&row["vendor"]));
            serde_json::from_value(row).context("converting derived row")
        })
        .collect()
}

pub fn load_rows(force_refresh: bool, cache_hours: f64) -> Result<(Vec<Row>, CacheState, String)> {
    let (cached, state) = cache::load_payloads()?;
    let age = cached["fetchedAt"]
        .as_str()
        .and_then(|stamp| OffsetDateTime::parse(stamp, &Rfc3339).ok())
        .map(|stamp| (OffsetDateTime::now_utc() - stamp).as_seconds_f64());
    let (payloads, state) =
        if !force_refresh && age.is_some_and(|age| age >= 0.0 && age < cache_hours * 3600.0) {
            (cached, state)
        } else {
            match cache::fetch_live() {
                Ok(fresh) => fresh,
                Err(_) => (cached, state),
            }
        };
    match rows_from_payloads(&payloads) {
        Ok(rows) => Ok((rows, state, label(&payloads["fetchedAt"]).to_owned())),
        Err(_) if state == CacheState::Disk => {
            let baked = cache::baked_payloads()?;
            Ok((
                rows_from_payloads(&baked)?,
                CacheState::Baked,
                label(&baked["fetchedAt"]).to_owned(),
            ))
        }
        Err(error) => Err(error),
    }
}
