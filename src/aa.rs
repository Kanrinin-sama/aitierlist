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

pub const AGENT_TASKS: [(&str, f64); 3] = [
    ("deep-swe", 113.0),
    ("terminal-bench-v2.1", 89.0),
    ("swe-atlas-qna", 124.0),
];
pub const BENCH_ATTEMPTS: [(&str, f64); 3] = [
    ("deep-swe", 3.0),
    ("terminal-bench-v2.1", 3.0),
    ("swe-atlas-qna", 3.0),
];
const QNA_TOTAL_TRIALS: f64 = 372.0;
const GPQA_TOTAL_TRIALS: f64 = 990.0;
const HLE_TOTAL_TRIALS: f64 = 2158.0;
const LCR_TOTAL_TRIALS: f64 = 300.0;
pub const EXCLUDED_EFFORTS: [&str; 1] = ["none"];
const EFFORT_ORDER: [&str; 6] = ["none", "low", "medium", "high", "xhigh", "max"];
const SITE: &str = "https://artificialanalysis.ai";
const QUALITY_FIELDS: [&str; 3] = ["swe", "term", "qna"];
const MEAN_FIELDS: [&str; 5] = [
    "costUsd",
    "agentWallTimeSec",
    "inputTokens",
    "outputTokens",
    "steps",
];

fn number(value: &Value) -> f64 {
    value.as_f64().unwrap_or(f64::NAN)
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

pub fn noise(pass: f64, task_count: f64) -> f64 {
    (pass * (1.0 - pass) / task_count).sqrt()
}

pub fn harness_key(name: &str) -> String {
    let name = name.to_lowercase();
    let name = regex(r"^claude\s+").replace(&name, "");
    regex(r"\s*\(with fallback\)")
        .replace(&name, "")
        .trim()
        .to_owned()
}

pub fn family_key(harness: &str, model_key: &str) -> String {
    format!(
        "{harness}|{}",
        regex(r"\s*\((?:max|xhigh|high|medium|low|none)\)$").replace(model_key, "")
    )
}

fn effort_of(model_key: &str) -> Option<String> {
    regex(r"\((max|xhigh|high|medium|low|none)\)$")
        .captures(model_key)
        .map(|capture| capture[1].to_owned())
}

pub fn display_name(raw_name: &str) -> String {
    let effort = regex(r"\(([^)]*)\)\s*$")
        .captures(raw_name)
        .and_then(|group| {
            regex(r"(?i)\b(max|xhigh|high|medium|low|none)\b")
                .captures(&group[1])
                .map(|capture| capture[1].to_lowercase())
        })
        .or_else(|| {
            raw_name
                .to_lowercase()
                .contains("non-reasoning")
                .then(|| "none".to_owned())
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
    let response = response.error_for_status()?;
    let etag = response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    Ok(Some(ResponseBody {
        bytes: response.bytes().await?.to_vec(),
        etag,
    }))
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

pub fn agent_row(raw: &Value, estimated: bool) -> Option<Value> {
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
    let implementation_mean =
        |key: &str| (number(&swe[key]) * 113.0 + number(&term[key]) * 89.0) / 202.0;
    let pass = implementation_mean("reward");
    if pass <= 0.0 {
        return None;
    }
    let wait = number(&mean["agentWallTimeSec"]) * implementation_mean("outputTokens")
        / number(&mean["outputTokens"]);
    let cost_scale = (implementation_mean("inputTokens") + implementation_mean("outputTokens"))
        / (number(&mean["inputTokens"]) + number(&mean["outputTokens"]));
    let read_seconds = number(&mean["agentWallTimeSec"]) * number(&qna["outputTokens"])
        / number(&mean["outputTokens"]);
    let read_usd = number(&mean["costUsd"])
        * (number(&qna["inputTokens"]) + number(&qna["outputTokens"]))
        / (number(&mean["inputTokens"]) + number(&mean["outputTokens"]));
    let time_rel = fallback(&mean["timeExtrapolationRel"], 0.0);
    let cost_rel = fallback(&mean["costExtrapolationRel"], 0.0);
    let model_key = harness_key(label(&raw["display"]["model"]));
    let harness = label(&raw["agentName"]);
    Some(json!({
        "name":clean_label(label(&raw["displayLabel"])),"rawName":raw["displayLabel"],"model":raw["display"]["model"],"harness":harness,"modelKey":model_key,"family":family_key(harness,&model_key),"vendor":vendor_key(harness,&model_key),"estimated":estimated,
        "swe":swe["reward"],"term":term["reward"],"qna":qna["reward"],"pass":pass,
        "benches":[{"benchmark":"deep-swe","tasks":113,"attempts":3,"pass":swe["reward"]},{"benchmark":"terminal-bench-v2.1","tasks":89,"attempts":3,"pass":term["reward"]}],
        "band":{"pass":(113.0 / 202.0 * noise(number(&swe["reward"]),339.0)).hypot(89.0 / 202.0 * noise(number(&term["reward"]),267.0))},
        "waitSeconds":wait,"waitSecondsBand":time_rel * wait,"attemptUsd":number(&mean["costUsd"]) * cost_scale,"readSeconds":read_seconds,"readSecondsBand":(read_seconds / QNA_TOTAL_TRIALS.sqrt()).hypot(time_rel * read_seconds),"readUsd":read_usd,"readUsdBand":cost_rel * read_usd,"timeExtrapolationRel":time_rel,"costExtrapolationRel":cost_rel,"usdPerStep":number(&mean["costUsd"]) / number(&mean["steps"]),"sourceMean":mean,"sourceEvaluations":evaluations
    }))
}

pub fn model_row(item: &Value, hosts: &[&Value]) -> Option<Value> {
    let tokens = &item["canonicalEvalTokenCounts"]["terminalbenchV21"];
    if item["deprecated"].as_bool().unwrap_or(false)
        || fallback(&item["terminalbenchV21"], 0.0) == 0.0
        || tokens.is_null()
        || item["gpqa"].is_null()
        || item["hle"].is_null()
    {
        return None;
    }
    let price = |key: &str| {
        item[key]
            .as_f64()
            .or_else(|| median(hosts.iter().map(|host| number(&host[key]))))
    };
    let input = price("price1mInputTokens");
    let output = price("price1mOutputTokens");
    let first_party: Vec<_> = hosts
        .iter()
        .copied()
        .filter(|host| host["host"]["name"] == item["creator"]["name"])
        .collect();
    let speed = median(
        if first_party.is_empty() {
            hosts
        } else {
            &first_party
        }
        .iter()
        .map(|host| number(&host["timescaleData"]["medianOutputSpeed"])),
    )
    .or_else(|| item["medianCanonicalAnswerOutputSpeed"].as_f64());
    let rankable = input.is_some()
        && output.is_some()
        && speed.is_some_and(|speed| speed != 0.0 && !speed.is_nan());
    let cache_price = price("cacheHitPrice");
    let cached = if cache_price.is_some() && present(&item["cacheHitRate"]) {
        fallback(&tokens["cacheableInput"], 0.0) * number(&item["cacheHitRate"])
    } else {
        0.0
    };
    let output_tokens = number(&tokens["answer"]) + number(&tokens["reasoning"]);
    let name = display_name(label(&item["name"]));
    let key = harness_key(&name);
    let mut row = json!({"name":name,"rawName":item["name"],"model":name,"modelKey":key,"harness":"model","family":family_key("model",&key),"vendor":vendor_key("model",&key),"intelligenceIndex":item["intelligenceIndex"],"term":item["terminalbenchV21"],"gpqa":item["gpqa"],"hle":item["hle"],"logic":(number(&item["gpqa"])+number(&item["hle"]))/2.0,"benches":[{"benchmark":"terminal-bench-v2.1","tasks":89,"attempts":3,"pass":item["terminalbenchV21"]}],"pass":item["terminalbenchV21"],"rankable":rankable,"speed":speed,"waitSeconds":if rankable { Some(output_tokens / 89.0 / speed.unwrap()) } else { None },"waitSecondsBand":0,"attemptUsd":if rankable { Some(((number(&tokens["input"])-cached)*input.unwrap()+cached*cache_price.unwrap_or(0.0)+output_tokens*output.unwrap())/1e6/89.0) } else { None }});
    apply_model_quality(&mut row, item);
    Some(row)
}

pub fn apply_model_quality(row: &mut Value, item: &Value) {
    row["index"] = json!(
        item["intelligenceIndex"]
            .as_f64()
            .map(|value| value / 100.0)
    );
    row["lcr"] = item["lcr"].clone();
    row["hallucination"] = item["omniscienceBreakdown"]["hallucinationRate"].clone();
    row["band"] = json!({"index":row["index"].as_f64().map(|_|0.0),"lcr":row["lcr"].as_f64().map(|value|noise(value, LCR_TOTAL_TRIALS)),"logic":(noise(number(&row["gpqa"]), GPQA_TOTAL_TRIALS)/2.0).hypot(noise(number(&row["hle"]), HLE_TOTAL_TRIALS)/2.0)});
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

pub fn robust_line(xs: &[f64], ys: &[f64]) -> (f64, f64) {
    let mut slopes = Vec::new();
    for left in 0..xs.len() {
        for right in left + 1..xs.len() {
            if xs[right] != xs[left] {
                slopes.push((ys[right] - ys[left]) / (xs[right] - xs[left]));
            }
        }
    }
    let slope = median(slopes).unwrap_or(1.0);
    (
        slope,
        median(ys.iter().zip(xs).map(|(y, x)| y - slope * x)).unwrap_or(0.0),
    )
}

fn anchor_for(rows: &[Value]) -> &Value {
    rows.iter()
        .reduce(|best, row| {
            let rank = |row: &Value| {
                effort_of(label(&row["modelKey"]))
                    .and_then(|effort| {
                        EFFORT_ORDER
                            .iter()
                            .position(|candidate| *candidate == effort)
                    })
                    .map(|index| index as i32)
                    .unwrap_or(-1)
            };
            if rank(row) > rank(best) { row } else { best }
        })
        .expect("nonempty family")
}

#[derive(Clone, Copy)]
struct EffortLevel {
    level: f64,
    spread: f64,
    donors: usize,
}

pub fn extrapolate_harness_rows(agents: &mut Vec<Value>, models: &[Value]) {
    let model_by_key: BTreeMap<_, _> = models
        .iter()
        .map(|row| (label(&row["modelKey"]), row))
        .collect();
    let mut families: Vec<(String, Vec<Value>)> = Vec::new();
    for row in agents.iter() {
        let family = label(&row["family"]);
        if let Some((_, rows)) = families.iter_mut().find(|(key, _)| key == family) {
            rows.push(row.clone());
        } else {
            families.push((family.to_owned(), vec![row.clone()]));
        }
    }
    let donors: Vec<_> = families
        .iter()
        .filter(|(_, rows)| rows.len() >= 2)
        .map(|(_, rows)| (rows, anchor_for(rows)))
        .collect();
    let intelligence = |row: &Value| {
        model_by_key
            .get(label(&row["modelKey"]))
            .and_then(|model| model["intelligenceIndex"].as_f64())
    };
    let fits: Vec<_> =
        QUALITY_FIELDS
            .iter()
            .map(|field| {
                let mut pairs = Vec::new();
                for (rows, anchor) in &donors {
                    for row in rows.iter() {
                        if std::ptr::eq(row, *anchor) {
                            continue;
                        }
                        if let (
                            Some(value),
                            Some(anchor_value),
                            Some(quality),
                            Some(anchor_quality),
                        ) = (
                            intelligence(row),
                            intelligence(anchor),
                            row[*field].as_f64(),
                            anchor[*field].as_f64(),
                        ) && anchor_value != 0.0
                            && anchor_quality != 0.0
                            && (value / anchor_value - 1.0).abs() > 1e-6
                        {
                            pairs.push((value / anchor_value, quality / anchor_quality));
                        }
                    }
                }
                let slope = median(pairs.iter().map(|(x, y)| (y - 1.0) / (x - 1.0))).unwrap_or(1.0);
                let residual = median(
                    pairs
                        .iter()
                        .map(|(x, y)| (y - (1.0 + slope * (x - 1.0))).abs()),
                )
                .unwrap_or(0.0);
                let distance = median(pairs.iter().map(|(x, _)| (x - 1.0).abs())).unwrap_or(0.0);
                (slope, residual, distance)
            })
            .collect();
    let levels: Vec<BTreeMap<&str, EffortLevel>> = MEAN_FIELDS
        .iter()
        .map(|field| {
            EFFORT_ORDER
                .iter()
                .filter_map(|effort| {
                    let ratios: Vec<_> = donors
                        .iter()
                        .filter_map(|(rows, anchor)| {
                            let row = rows.iter().find(|row| {
                                effort_of(label(&row["modelKey"])).as_deref() == Some(*effort)
                            })?;
                            let value = row["sourceMean"][*field].as_f64()?;
                            let anchor_value = anchor["sourceMean"][*field].as_f64()?;
                            (anchor_value != 0.0).then_some(value / anchor_value)
                        })
                        .collect();
                    let level = median(ratios.iter().copied())?;
                    Some((
                        *effort,
                        EffortLevel {
                            level,
                            spread: median(ratios.iter().map(|ratio| (ratio - level).abs()))
                                .unwrap_or(0.0),
                            donors: ratios.len(),
                        },
                    ))
                })
                .collect()
        })
        .collect();
    for (_, rows) in &families {
        let anchor = anchor_for(rows);
        let base = family_key("", label(&anchor["modelKey"]));
        let Some(anchor_intelligence) = intelligence(anchor).filter(|value| *value != 0.0) else {
            continue;
        };
        let Some(anchor_effort) = effort_of(label(&anchor["modelKey"])) else {
            continue;
        };
        for model in models
            .iter()
            .filter(|model| family_key("", label(&model["modelKey"])) == base)
        {
            let Some(effort) = effort_of(label(&model["modelKey"])) else {
                continue;
            };
            if EXCLUDED_EFFORTS.contains(&effort.as_str())
                || rows
                    .iter()
                    .any(|row| effort_of(label(&row["modelKey"])).as_deref() == Some(&effort))
            {
                continue;
            }
            let Some(model_intelligence) = model["intelligenceIndex"].as_f64() else {
                continue;
            };
            let ratios: Option<Vec<_>> = levels
                .iter()
                .map(|levels| {
                    let current = levels.get(effort.as_str())?;
                    let anchor = levels.get(anchor_effort.as_str())?;
                    (current.level != 0.0 && anchor.level != 0.0)
                        .then_some(current.level / anchor.level)
                })
                .collect();
            let Some(ratios) = ratios else {
                continue;
            };
            let relative = |index: usize| {
                let current = levels[index][effort.as_str()];
                let anchor = levels[index][anchor_effort.as_str()];
                let relative = |level: &EffortLevel| {
                    (level.spread / level.level).hypot(anchor.spread / anchor.level)
                };
                (if current.donors > 1 && anchor.donors > 1 {
                    relative(&current)
                } else {
                    median(
                        levels[index]
                            .values()
                            .filter(|level| level.donors > 1 && anchor.donors > 1)
                            .map(relative),
                    )
                    .unwrap_or(0.0)
                }) / 0.6745
            };
            let mut mean = json!({});
            for (index, field) in MEAN_FIELDS.iter().enumerate() {
                mean[*field] = json!(
                    anchor["sourceMean"][*field]
                        .as_f64()
                        .map(|value| value * ratios[index])
                );
            }
            mean["timeExtrapolationRel"] = json!(relative(1));
            mean["costExtrapolationRel"] = json!(relative(0));
            let mut evals = Vec::new();
            let mut bands = json!({});
            for (index, field) in QUALITY_FIELDS.iter().enumerate() {
                let (slope, residual, typical_distance) = fits[index];
                let ratio = model_intelligence / anchor_intelligence;
                let quality =
                    (number(&anchor[*field]) * (1.0 + slope * (ratio - 1.0))).clamp(0.0, 1.0);
                let dataset = AGENT_TASKS[index].0;
                let mut source = anchor["sourceEvaluations"][dataset].clone();
                source["reward"] = json!(quality);
                for field in ["inputTokens", "outputTokens"] {
                    source[field] = json!(
                        number(&source[field]) * number(&mean[field])
                            / number(&anchor["sourceMean"][field])
                    );
                }
                evals.push(json!({"datasetIndexName":dataset,"mean":source}));
                let scale = if typical_distance > 0.0 {
                    ((ratio - 1.0).abs() / typical_distance).max(1.0)
                } else {
                    1.0
                };
                bands[*field] = json!(number(&anchor[*field]) * residual * scale);
            }
            let raw = json!({"isUnavailable":false,"displayLabel":format!("~{} - {}",label(&anchor["harness"]),label(&model["name"])),"agentName":anchor["harness"],"display":{"model":model["name"]},"mean":mean,"evals":evals});
            if let Some(mut row) = agent_row(&raw, true) {
                row["modelKey"] = model["modelKey"].clone();
                row["extrapolationBand"] = bands;
                agents.push(row);
            }
        }
    }
}

pub fn apply_harness_line(agents: &mut [Value], models: &mut [Value]) {
    let model_by_key: BTreeMap<_, _> = models
        .iter()
        .map(|row| (label(&row["modelKey"]), row))
        .collect();
    for row in agents.iter_mut() {
        let model = model_by_key
            .get(label(&row["modelKey"]))
            .copied()
            .unwrap_or(&Value::Null);
        for field in ["index", "lcr", "logic", "hallucination"] {
            row[field] = model[field].clone();
        }
        for field in ["index", "lcr", "logic"] {
            row["band"][field] = model["band"][field].clone();
        }
    }
    let decode: BTreeMap<_, _> = models
        .iter()
        .filter(|row| row["rankable"] == true)
        .map(|row| (label(&row["modelKey"]), number(&row["waitSeconds"])))
        .collect();
    let matched: Vec<_> = agents
        .iter()
        .filter(|row| decode.contains_key(label(&row["modelKey"])))
        .collect();
    let xs: Vec<_> = matched
        .iter()
        .map(|row| decode[label(&row["modelKey"])])
        .collect();
    let ys: Vec<_> = matched
        .iter()
        .map(|row| number(&row["waitSeconds"]))
        .collect();
    let (slope, intercept) = robust_line(&xs, &ys);
    let predict = |value: f64| slope * value + intercept;
    let mut ratios: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for row in &matched {
        ratios
            .entry(label(&row["harness"]))
            .or_default()
            .push(number(&row["waitSeconds"]) / predict(decode[label(&row["modelKey"])]));
    }
    let mut measured: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for row in &matched {
        measured
            .entry(label(&row["modelKey"]).to_owned())
            .or_default()
            .push(
                number(&row["waitSeconds"])
                    / median(ratios[label(&row["harness"])].iter().copied()).unwrap_or(f64::NAN),
            );
    }
    let residual = median(xs.iter().zip(&ys).map(|(x, y)| (y - predict(*x)).abs())).unwrap_or(0.0);
    for row in models.iter_mut().filter(|row| row["rankable"] == true) {
        let measurement = measured
            .get(label(&row["modelKey"]))
            .and_then(|values| median(values.iter().copied()));
        row["waitSeconds"] = json!(measurement.unwrap_or_else(|| {
            number(&row["waitSeconds"]).max(predict(number(&row["waitSeconds"])))
        }));
        row["waitSecondsBand"] = json!(if measurement.is_some() { 0.0 } else { residual });
    }
}

pub fn shrink_values(rows: &mut [Value], path: &str, task_count: f64) {
    let passes: Vec<_> = rows
        .iter()
        .filter(|row| row["extrapolationBand"].is_null())
        .filter_map(|row| row.pointer(path).and_then(Value::as_f64))
        .collect();
    if passes.is_empty() {
        return;
    }
    let mean = passes.iter().sum::<f64>() / passes.len() as f64;
    let variance =
        passes.iter().map(|pass| (pass - mean).powi(2)).sum::<f64>() / passes.len() as f64;
    let noise_variance = passes
        .iter()
        .map(|pass| pass * (1.0 - pass) / task_count)
        .sum::<f64>()
        / passes.len() as f64;
    let true_variance = (variance - noise_variance).max(1e-9);
    let prior_weight = (mean * (1.0 - mean) / true_variance - 1.0).max(0.0);
    for row in rows {
        if let Some(value) = row.pointer_mut(path).filter(|value| value.is_number()) {
            let pass = number(value);
            let shrunk = (task_count * pass + prior_weight * mean) / (task_count + prior_weight);
            let adjustment = shrunk - pass;
            *value =
                json!(pass + adjustment.signum() * adjustment.abs().min(noise(pass, task_count)));
        }
    }
}

pub fn shrink_quality_fields(rows: &mut [Value]) {
    if rows.is_empty() {
        return;
    }
    let benches = array(&rows[0]["benches"]).to_vec();
    for (index, bench) in benches.iter().enumerate() {
        shrink_values(
            rows,
            &format!("/benches/{index}/pass"),
            number(&bench["tasks"]) * number(&bench["attempts"]),
        );
    }
    for row in rows.iter_mut() {
        let total: f64 = array(&row["benches"])
            .iter()
            .map(|bench| number(&bench["tasks"]))
            .sum();
        row["pass"] = json!(
            array(&row["benches"])
                .iter()
                .map(|bench| number(&bench["tasks"]) / total * number(&bench["pass"]))
                .sum::<f64>()
        );
        let measurement: f64 = array(&row["benches"])
            .iter()
            .map(|bench| {
                (number(&bench["tasks"]) / total).powi(2)
                    * number(&bench["pass"])
                    * (1.0 - number(&bench["pass"]))
                    / (number(&bench["tasks"]) * number(&bench["attempts"]))
            })
            .sum();
        let extrapolation = if row["extrapolationBand"].is_null() {
            0.0
        } else {
            (number(&row["extrapolationBand"]["swe"]) * 113.0 / 202.0).powi(2)
                + (number(&row["extrapolationBand"]["term"]) * 89.0 / 202.0).powi(2)
        };
        row["band"]["pass"] = json!((measurement + extrapolation).sqrt());
    }
    for (field, tasks) in [
        ("qna", QNA_TOTAL_TRIALS),
        ("gpqa", GPQA_TOTAL_TRIALS),
        ("hle", HLE_TOTAL_TRIALS),
    ] {
        if present(&rows[0][field]) {
            shrink_values(rows, &format!("/{field}"), tasks);
        }
    }
    for row in rows {
        let has_logic = row["gpqa"].is_number() && row["hle"].is_number();
        if has_logic {
            row["logic"] = json!((number(&row["gpqa"]) + number(&row["hle"])) / 2.0);
        }
        let qna_band = row["qna"].as_f64().map(|qna| {
            noise(qna, QNA_TOTAL_TRIALS).hypot(fallback(&row["extrapolationBand"]["qna"], 0.0))
        });
        let lcr_band = row["lcr"].as_f64().map(|lcr| noise(lcr, LCR_TOTAL_TRIALS));
        let logic_band = if has_logic {
            Some(
                (noise(number(&row["gpqa"]), GPQA_TOTAL_TRIALS) / 2.0)
                    .hypot(noise(number(&row["hle"]), HLE_TOTAL_TRIALS) / 2.0),
            )
        } else {
            row["band"]["logic"].as_f64()
        };
        row["band"]["index"] = json!(row["index"].as_f64().map(|_| 0.0));
        row["band"]["lcr"] = json!(lcr_band);
        row["band"]["logic"] = json!(logic_band);
        row["sanityQuality"] = row["qna"].clone();
        row["band"]["sanityQuality"] = json!(qna_band);
        row["orchQuality"] = json!(
            row["index"]
                .as_f64()
                .zip(row["logic"].as_f64())
                .map(|(index, logic)| (index + logic) / 2.0)
        );
        row["band"]["orchQuality"] = json!(
            row["orchQuality"]
                .as_f64()
                .and(logic_band.map(|band| band / 2.0))
        );
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
    let mut agents: Vec<_> = agents
        .iter()
        .filter_map(|raw| agent_row(raw, false))
        .collect();
    let mut models: Vec<_> = evaluation
        .iter()
        .filter_map(|item| {
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
    extrapolate_harness_rows(&mut agents, &models);
    models.retain(|row| row["rankable"] == true);
    shrink_quality_fields(&mut models);
    apply_harness_line(&mut agents, &mut models);
    shrink_quality_fields(&mut agents);
    agents.extend(models);
    agents
        .into_iter()
        .map(|mut row| {
            row["effort"] = json!(effort_of(label(&row["modelKey"])));
            row["model"] = json!(
                regex(r"\s*\((?:max|xhigh|high|medium|low|none)\)$")
                    .replace(label(&row["model"]), "")
                    .to_string()
            );
            row["vendor"] = json!(label(&row["vendor"]));
            row["attemptUsdBand"] =
                json!(fallback(&row["costExtrapolationRel"], 0.0) * number(&row["attemptUsd"]));
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
    Ok((
        rows_from_payloads(&payloads)?,
        state,
        label(&payloads["fetchedAt"]).to_owned(),
    ))
}
