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

const QNA_TOTAL_TRIALS: f64 = 372.0;
const GPQA_TOTAL_TRIALS: f64 = 990.0;
const HLE_TOTAL_TRIALS: f64 = 2158.0;
const LCR_TOTAL_TRIALS: f64 = 300.0;
pub const EXCLUDED_EFFORTS: [&str; 1] = ["none"];
const SITE: &str = "https://artificialanalysis.ai";

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

pub fn agent_row(raw: &Value) -> Option<Value> {
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
    let attempt_usd = number(&mean["costUsd"]) * cost_scale;
    let model_key = harness_key(label(&raw["display"]["model"]));
    let harness = label(&raw["agentName"]);
    Some(json!({
        "name":clean_label(label(&raw["displayLabel"])),"rawName":raw["displayLabel"],"model":raw["display"]["model"],"harness":harness,"modelKey":model_key,"family":family_key(harness,&model_key),"vendor":vendor_key(harness,&model_key),
        "swe":swe["reward"],"term":term["reward"],"qna":qna["reward"],"pass":pass,
        "benches":[{"benchmark":"deep-swe","tasks":113,"attempts":3,"pass":swe["reward"]},{"benchmark":"terminal-bench-v2.1","tasks":89,"attempts":3,"pass":term["reward"]}],
        "band":{},
        "waitSeconds":wait,"waitSecondsBand":wait / 202.0_f64.sqrt(),"attemptUsd":attempt_usd,"attemptUsdBand":attempt_usd / 202.0_f64.sqrt(),"readSeconds":read_seconds,"readSecondsBand":read_seconds / QNA_TOTAL_TRIALS.sqrt(),"readUsd":read_usd,"readUsdBand":read_usd / QNA_TOTAL_TRIALS.sqrt(),"usdPerStep":number(&mean["costUsd"]) / number(&mean["steps"])
    }))
}

pub fn model_row(item: &Value, hosts: &[&Value]) -> Value {
    let tokens = &item["canonicalEvalTokenCounts"]["terminalbenchV21"];
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
    let rankable = !item["deprecated"].as_bool().unwrap_or(false)
        && fallback(&item["terminalbenchV21"], 0.0) > 0.0
        && !tokens.is_null()
        && item["gpqa"].is_number()
        && item["hle"].is_number()
        && input.is_some()
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
    let key = harness_key(label(&item["name"]));
    let mut row = json!({"name":name,"rawName":item["name"],"model":name,"modelKey":key,"harness":"model","family":family_key("model",&key),"vendor":vendor_key("model",&key),"term":item["terminalbenchV21"],"gpqa":item["gpqa"],"hle":item["hle"],"logic":(number(&item["gpqa"])+number(&item["hle"]))/2.0,"benches":[{"benchmark":"terminal-bench-v2.1","tasks":89,"attempts":3,"pass":item["terminalbenchV21"]}],"pass":item["terminalbenchV21"],"rankable":rankable,"speed":speed,"waitSeconds":if rankable { Some(output_tokens / 89.0 / speed.unwrap()) } else { None },"waitSecondsBand":0,"attemptUsd":if rankable { Some(((number(&tokens["input"])-cached)*input.unwrap()+cached*cache_price.unwrap_or(0.0)+output_tokens*output.unwrap())/1e6/89.0) } else { None }});
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
    row["lcr"] = item["lcr"].clone();
    row["hallucination"] = item["omniscienceBreakdown"]["hallucinationRate"].clone();
    row["band"] = json!({"index":row["index"].as_f64().map(|_|0.0),"lcr":row["lcr"].as_f64().map(|value|noise(value, LCR_TOTAL_TRIALS)),"gpqa":row["gpqa"].as_f64().map(|value|noise(value, GPQA_TOTAL_TRIALS)),"logic":(noise(number(&row["gpqa"]), GPQA_TOTAL_TRIALS)/2.0).hypot(noise(number(&row["hle"]), HLE_TOTAL_TRIALS)/2.0)});
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

pub fn apply_harness_line(agents: &mut [Value], models: &mut [Value]) {
    let mut model_by_key: BTreeMap<&str, &Value> = BTreeMap::new();
    for row in models.iter() {
        model_by_key
            .entry(label(&row["modelKey"]))
            .and_modify(|existing| {
                if existing["deprecated"] == true && row["deprecated"] != true {
                    *existing = row;
                }
            })
            .or_insert(row);
    }
    let mut model_by_family = BTreeMap::new();
    for (key, model) in &model_by_key {
        model_by_family
            .entry(family_key("", key))
            .and_modify(|entry| *entry = None)
            .or_insert(Some(*model));
    }
    for row in agents.iter_mut() {
        let key = label(&row["modelKey"]);
        let model = model_by_key
            .get(key)
            .copied()
            .or_else(|| {
                if effort_of(key).is_none() {
                    model_by_family.get(&family_key("", key)).copied().flatten()
                } else {
                    None
                }
            })
            .unwrap_or(&Value::Null);
        for field in ["index", "lcr", "logic", "hallucination", "gpqa"] {
            row[field] = model[field].clone();
        }
        for field in ["index", "lcr", "logic", "gpqa"] {
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
        .filter(|row| row["rankable"] != false)
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
        row["band"]["pass"] = json!(measurement.sqrt());
    }
    for (field, tasks) in [
        ("qna", QNA_TOTAL_TRIALS),
        ("gpqa", GPQA_TOTAL_TRIALS),
        ("hle", HLE_TOTAL_TRIALS),
    ] {
        if field == "qna" || rows[0]["harness"] == "model" {
            shrink_values(rows, &format!("/{field}"), tasks);
        }
    }
    for row in rows {
        let has_logic = row["gpqa"].is_number() && row["hle"].is_number();
        if has_logic {
            row["logic"] = json!((number(&row["gpqa"]) + number(&row["hle"])) / 2.0);
        }
        let qna_band = row["qna"].as_f64().map(|qna| noise(qna, QNA_TOTAL_TRIALS));
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
        row["band"]["sanityQuality"] = json!(qna_band);
        row["band"]["gpqa"] = json!(
            row["gpqa"]
                .as_f64()
                .map(|gpqa| noise(gpqa, GPQA_TOTAL_TRIALS))
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
    let mut agents: Vec<_> = agents.iter().filter_map(agent_row).collect();
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
    shrink_quality_fields(&mut models);
    apply_harness_line(&mut agents, &mut models);
    models.retain(|row| row["rankable"] == true);
    shrink_quality_fields(&mut agents);
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
