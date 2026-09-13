use crate::evidence::{SourceRef, SourceSnapshot};
use crate::upstream::{deepswe, epoch, hle, scale, surge, terminal};
use reqwest::header::ETAG;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::sync::Arc;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

type Parser = fn(SourceRef, Value) -> anyhow::Result<SourceSnapshot>;

#[derive(Clone, Copy)]
enum PayloadKind {
    Json,
    Html,
    EpochArchive,
}

#[derive(Clone, Copy)]
struct Feed {
    source_id: &'static str,
    url: &'static str,
    payload: PayloadKind,
    parser: Parser,
    observations_required: bool,
}

const FEEDS: &[Feed] = &[
    deep(
        "deep-swe-v1.1-leaderboard",
        "https://deepswe.datacurve.ai/artifacts/v1.1/leaderboard-live.json",
        true,
    ),
    deep(
        "deep-swe-v1.1-trials",
        "https://deepswe.datacurve.ai/artifacts/v1.1/trials.json",
        false,
    ),
    deep(
        "deep-swe-v1.1-tasks",
        "https://deepswe.datacurve.ai/artifacts/v1.1/tasks.json",
        false,
    ),
    deep(
        "deep-swe-v1-leaderboard",
        "https://deepswe.datacurve.ai/artifacts/v1/leaderboard-live.json",
        true,
    ),
    deep(
        "deep-swe-v1-trials",
        "https://deepswe.datacurve.ai/artifacts/v1/trials.json",
        false,
    ),
    deep(
        "deep-swe-v1-tasks",
        "https://deepswe.datacurve.ai/artifacts/v1/tasks.json",
        false,
    ),
    html(
        "scale-sweatlas-qna",
        "https://labs.scale.com/leaderboard/sweatlas-qna",
        scale::parse,
    ),
    html(
        "scale-sweatlas-tw",
        "https://labs.scale.com/leaderboard/sweatlas-tw",
        scale::parse,
    ),
    html(
        "scale-sweatlas-refactoring",
        "https://labs.scale.com/leaderboard/sweatlas-refactoring",
        scale::parse,
    ),
    html(
        "scale-mcp-atlas",
        "https://labs.scale.com/leaderboard/mcp_atlas",
        scale::parse,
    ),
    html("hle-original", "https://lastexam.ai/", hle::parse),
    html(
        "surge-gdp-pdf",
        "https://surgehq.ai/benchmarks/gdp-pdf",
        surge::parse,
    ),
    html(
        "surge-chartography",
        "https://surgehq.ai/benchmarks/chartography",
        surge::parse,
    ),
    html(
        "surge-hemingway",
        "https://surgehq.ai/benchmarks/hemingway-bench",
        surge::parse,
    ),
    Feed {
        source_id: "epoch-benchmark-data",
        url: "https://epoch.ai/data/benchmark_data.zip",
        payload: PayloadKind::EpochArchive,
        parser: epoch::parse,
        observations_required: true,
    },
];

const fn deep(source_id: &'static str, url: &'static str, observations_required: bool) -> Feed {
    Feed {
        source_id,
        url,
        payload: PayloadKind::Json,
        parser: deepswe::parse,
        observations_required,
    }
}

const fn html(source_id: &'static str, url: &'static str, parser: Parser) -> Feed {
    Feed {
        source_id,
        url,
        payload: PayloadKind::Html,
        parser,
        observations_required: true,
    }
}

pub async fn refresh(client: &reqwest::Client, previous: &[SourceSnapshot]) -> Vec<SourceSnapshot> {
    let mut pending = tokio::task::JoinSet::new();
    let permits = Arc::new(tokio::sync::Semaphore::new(4));
    for feed in FEEDS {
        let client = client.clone();
        let feed = *feed;
        let permits = permits.clone();
        pending.spawn(async move {
            let _permit = permits.acquire_owned().await.map_err(|error| Failure {
                source: source(
                    feed.source_id,
                    feed.url,
                    String::new(),
                    now(),
                    &reqwest::header::HeaderMap::new(),
                ),
                error: error.to_string(),
            })?;
            refresh_feed(&client, feed).await
        });
    }
    for terminal_feed in terminal_feeds() {
        let client = client.clone();
        let permits = permits.clone();
        pending.spawn(async move {
            let _permit = permits.acquire_owned().await.map_err(|error| Failure {
                source: source(
                    terminal_feed.source_id,
                    "",
                    String::new(),
                    now(),
                    &reqwest::header::HeaderMap::new(),
                ),
                error: error.to_string(),
            })?;
            refresh_terminal(&client, terminal_feed).await
        });
    }
    let mut snapshots = Vec::with_capacity(FEEDS.len() + 3);
    while let Some(result) = pending.join_next().await {
        match result {
            Ok(Ok(snapshot)) => snapshots.push(snapshot),
            Ok(Err(failure)) => snapshots.push(last_good(previous, *failure)),
            Err(error) => snapshots.push(failed_source("upstream-task", "", error.to_string())),
        }
    }
    snapshots.push(unavailable(
        previous,
        "hle-rolling",
        "https://agi.safe.ai/",
        "HLE-Rolling has no confirmed public model-results feed",
    ));
    snapshots.sort_by(|left, right| left.source.source_id.cmp(&right.source.source_id));
    snapshots
}

struct Failure {
    source: SourceRef,
    error: String,
}

type RefreshResult = Result<SourceSnapshot, Box<Failure>>;

async fn refresh_feed(client: &reqwest::Client, feed: Feed) -> RefreshResult {
    let fetched_at = now();
    let response = client
        .get(feed.url)
        .header("User-Agent", "aitierlist")
        .send()
        .await
        .map_err(|error| failure(feed.source_id, feed.url, fetched_at.clone(), error))?;
    let response = response
        .error_for_status()
        .map_err(|error| failure(feed.source_id, feed.url, fetched_at.clone(), error))?;
    let headers = response.headers().clone();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| failure(feed.source_id, feed.url, fetched_at.clone(), error))?;
    let revision = revision(&headers, &bytes);
    let source = source(feed.source_id, feed.url, revision, fetched_at, &headers);
    let payload = decode(feed.payload, &bytes).map_err(|error| Failure {
        source: source.clone(),
        error: error.to_string(),
    })?;
    let snapshot = (feed.parser)(source.clone(), payload).map_err(|error| Failure {
        source,
        error: error.to_string(),
    })?;
    if feed.observations_required && snapshot.observations.is_empty() {
        return Err(Failure {
            source: snapshot.source,
            error: "rankable source produced no observations".to_owned(),
        }
        .into());
    }
    Ok(snapshot)
}

#[derive(Clone, Copy)]
struct TerminalFeed {
    source_id: &'static str,
    repository: &'static str,
}

fn terminal_feeds() -> [TerminalFeed; 3] {
    [
        TerminalFeed {
            source_id: "terminal-bench-v4",
            repository: "harbor-framework/terminal-bench",
        },
        TerminalFeed {
            source_id: "terminal-bench-v2.1",
            repository: "harbor-framework/terminal-bench-2-1",
        },
        TerminalFeed {
            source_id: "terminal-science-v0.1",
            repository: "harbor-framework/terminal-bench-science",
        },
    ]
}

async fn refresh_terminal(client: &reqwest::Client, feed: TerminalFeed) -> RefreshResult {
    let fetched_at = now();
    let repository_url = format!("https://api.github.com/repos/{}", feed.repository);
    let repository = fetch_json(client, &repository_url)
        .await
        .map_err(|error| Failure {
            source: source(
                feed.source_id,
                &repository_url,
                String::new(),
                fetched_at.clone(),
                &reqwest::header::HeaderMap::new(),
            ),
            error,
        })?;
    let default_branch = repository["default_branch"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure {
            source: source(
                feed.source_id,
                &repository_url,
                String::new(),
                fetched_at.clone(),
                &reqwest::header::HeaderMap::new(),
            ),
            error: "GitHub repository has no default branch".to_owned(),
        })?;
    let head_url = format!(
        "https://api.github.com/repos/{}/commits/{default_branch}",
        feed.repository
    );
    let head = fetch_json(client, &head_url)
        .await
        .map_err(|error| Failure {
            source: source(
                feed.source_id,
                &head_url,
                String::new(),
                fetched_at.clone(),
                &reqwest::header::HeaderMap::new(),
            ),
            error,
        })?;
    let revision = head["sha"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| Failure {
            source: source(
                feed.source_id,
                &head_url,
                String::new(),
                fetched_at.clone(),
                &reqwest::header::HeaderMap::new(),
            ),
            error: "GitHub default-branch head has no commit SHA".to_owned(),
        })?;
    let tree_url = format!(
        "https://api.github.com/repos/{}/git/trees/{revision}?recursive=1",
        feed.repository
    );
    let source = source(
        feed.source_id,
        &tree_url,
        revision.to_owned(),
        fetched_at,
        &reqwest::header::HeaderMap::new(),
    );
    let tree = fetch_json(client, &tree_url)
        .await
        .map_err(|error| Failure {
            source: source.clone(),
            error,
        })?;
    let paths = tree["tree"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["path"].as_str())
        .filter(|path| {
            let lower = path.to_ascii_lowercase();
            (lower.starts_with("leaderboard/submissions/") && lower.ends_with(".json"))
                || (lower.starts_with("leaderboard/runs/") && lower.ends_with(".json"))
                || lower == "leaderboard/leaderboard.yaml"
        })
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if !paths.iter().any(|path| {
        path.to_ascii_lowercase()
            .starts_with("leaderboard/submissions/")
    }) {
        return Err(Failure {
            source,
            error: "Terminal-Bench tree has no submission JSON".to_owned(),
        }
        .into());
    }
    let mut submissions = Vec::new();
    let mut raw_files = Map::new();
    for path in paths {
        let url = format!(
            "https://raw.githubusercontent.com/{}/{revision}/{}",
            feed.repository, path
        );
        let payload = if path.to_ascii_lowercase().ends_with(".json") {
            fetch_json(client, &url).await.map_err(|error| Failure {
                source: source.clone(),
                error,
            })?
        } else {
            Value::String(fetch_text(client, &url).await.map_err(|error| Failure {
                source: source.clone(),
                error,
            })?)
        };
        if path
            .to_ascii_lowercase()
            .starts_with("leaderboard/submissions/")
        {
            collect_terminal_submissions(payload.clone(), &path, &url, &mut submissions);
        }
        raw_files.insert(path, payload);
    }
    let payload = Value::Object(Map::from_iter([
        ("repository".to_owned(), repository),
        ("head".to_owned(), head),
        ("submissions".to_owned(), Value::Array(submissions)),
        ("rawFiles".to_owned(), Value::Object(raw_files)),
    ]));
    let snapshot = terminal::parse(source.clone(), payload).map_err(|error| Failure {
        source,
        error: error.to_string(),
    })?;
    if snapshot.observations.is_empty() {
        return Err(Failure {
            source: snapshot.source,
            error: "Terminal-Bench source produced no observations".to_owned(),
        }
        .into());
    }
    Ok(snapshot)
}

fn collect_terminal_submissions(
    payload: Value,
    path: &str,
    url: &str,
    submissions: &mut Vec<Value>,
) {
    if let Some(values) = payload.as_array() {
        submissions.extend(
            values
                .iter()
                .filter(|value| value.get("metadata").is_some() && value.get("metrics").is_some())
                .cloned()
                .map(|value| terminal_provenance(value, path, url)),
        );
    } else if let Some(values) = payload.get("submissions").and_then(Value::as_array) {
        submissions.extend(
            values
                .iter()
                .cloned()
                .map(|value| terminal_provenance(value, path, url)),
        );
    } else if let Some(submission) = payload.get("submission") {
        submissions.push(terminal_provenance(submission.clone(), path, url));
    } else if payload.get("metadata").is_some() && payload.get("metrics").is_some() {
        submissions.push(terminal_provenance(payload, path, url));
    }
}

fn terminal_provenance(mut submission: Value, path: &str, url: &str) -> Value {
    if let Some(object) = submission.as_object_mut() {
        object.insert(
            "_submission_path".to_owned(),
            Value::String(path.to_owned()),
        );
        object.insert("_submission_url".to_owned(), Value::String(url.to_owned()));
    }
    submission
}

async fn fetch_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    let bytes = client
        .get(url)
        .header("User-Agent", "aitierlist")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .bytes()
        .await
        .map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    client
        .get(url)
        .header("User-Agent", "aitierlist")
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .text()
        .await
        .map_err(|error| error.to_string())
}

fn decode(kind: PayloadKind, bytes: &[u8]) -> anyhow::Result<Value> {
    match kind {
        PayloadKind::Json => Ok(serde_json::from_slice(bytes)?),
        PayloadKind::Html => Ok(Value::String(String::from_utf8(bytes.to_vec())?)),
        PayloadKind::EpochArchive => decode_epoch(bytes),
    }
}

fn decode_epoch(bytes: &[u8]) -> anyhow::Result<Value> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut files = Map::new();
    let mut metadata = Vec::new();
    for index in 0..archive.len() {
        let mut file = archive.by_index(index)?;
        if file.is_dir() || !file.name().to_ascii_lowercase().ends_with(".csv") {
            continue;
        }
        let name = file
            .name()
            .rsplit('/')
            .next()
            .unwrap_or(file.name())
            .to_owned();
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;
        let rows = csv_rows(contents.as_bytes())?;
        if name.ends_with("benchmark_metadata.csv") {
            metadata = rows;
        } else {
            files.insert(name, Value::Array(rows));
        }
    }
    Ok(Value::Object(Map::from_iter([
        ("metadata".to_owned(), Value::Array(metadata)),
        ("files".to_owned(), Value::Object(files)),
    ])))
}

fn csv_rows(bytes: &[u8]) -> anyhow::Result<Vec<Value>> {
    let mut reader = csv::Reader::from_reader(bytes);
    let headers = reader.headers()?.clone();
    reader
        .records()
        .map(|record| {
            let record = record?;
            Ok(Value::Object(
                headers
                    .iter()
                    .zip(record.iter())
                    .map(|(key, value)| (key.to_owned(), Value::String(value.to_owned())))
                    .collect(),
            ))
        })
        .collect()
}

fn source(
    source_id: &str,
    url: &str,
    revision: String,
    fetched_at: String,
    _headers: &reqwest::header::HeaderMap,
) -> SourceRef {
    SourceRef {
        source_id: source_id.to_owned(),
        url: url.to_owned(),
        revision,
        fetched_at,
        observed_at: None,
        published_at: None,
        license: None,
    }
}

fn revision(headers: &reqwest::header::HeaderMap, bytes: &[u8]) -> String {
    headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_matches('"').to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| hex::encode(Sha256::digest(bytes)))
}

fn failure(
    source_id: &str,
    url: &str,
    fetched_at: String,
    error: impl std::fmt::Display,
) -> Failure {
    Failure {
        source: source(
            source_id,
            url,
            String::new(),
            fetched_at,
            &reqwest::header::HeaderMap::new(),
        ),
        error: error.to_string(),
    }
}

fn last_good(previous: &[SourceSnapshot], failure: Failure) -> SourceSnapshot {
    if let Some(previous) = previous.iter().find(|snapshot| {
        snapshot.source.source_id == failure.source.source_id && snapshot.payload.is_some()
    }) {
        let mut snapshot = previous.clone();
        snapshot.fetch_error = Some(failure.error);
        snapshot.last_good_revision = Some(previous.source.revision.clone());
        snapshot.last_good_fetched_at = Some(previous.source.fetched_at.clone());
        return snapshot;
    }
    SourceSnapshot {
        source: failure.source,
        payload: None,
        observations: Vec::new(),
        fetch_error: Some(failure.error),
        last_good_revision: None,
        last_good_fetched_at: None,
    }
}

fn failed_source(source_id: &str, url: &str, error: String) -> SourceSnapshot {
    SourceSnapshot {
        source: source(
            source_id,
            url,
            String::new(),
            now(),
            &reqwest::header::HeaderMap::new(),
        ),
        payload: None,
        observations: Vec::new(),
        fetch_error: Some(error),
        last_good_revision: None,
        last_good_fetched_at: None,
    }
}

fn unavailable(
    previous: &[SourceSnapshot],
    source_id: &str,
    url: &str,
    error: &str,
) -> SourceSnapshot {
    last_good(
        previous,
        Failure {
            source: source(
                source_id,
                url,
                String::new(),
                now(),
                &reqwest::header::HeaderMap::new(),
            ),
            error: error.to_owned(),
        },
    )
}

fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}
