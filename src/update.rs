use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::io::Seek;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub const ALLOWED_HOST: &str = "files.blockitall.us";
pub const ARTIFACT_PREFIX: &str = "/aitierlist/releases/";
const MANIFEST_URL: &str = "https://files.blockitall.us/aitierlist/update.json";

#[used]
pub static COMMIT_TAG: &str = concat!("AITIERLIST_COMMIT=", env!("AITIERLIST_COMMIT"), "\0");

#[used]
pub static TREE_TAG: &str = concat!("AITIERLIST_TREE=", env!("AITIERLIST_TREE"), "\0");

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
pub const MAX_ARTIFACT_BYTES: u64 = 512 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: usize = 4;

const CHECK_BUDGET: Duration = Duration::from_secs(120);
const ARTIFACT_BUDGET: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Copy)]
pub struct Budget {
    started: Instant,
    limit: Duration,
    what: &'static str,
}

impl Budget {
    pub fn new(limit: Duration, what: &'static str) -> Self {
        Budget {
            started: Instant::now(),
            limit,
            what,
        }
    }

    fn elapsed_at(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.started)
    }

    fn exhausted_at(&self, now: Instant) -> bool {
        self.elapsed_at(now) >= self.limit
    }

    fn remaining_at(&self, now: Instant) -> Duration {
        self.limit.saturating_sub(self.elapsed_at(now))
    }

    fn check(&self) -> Result<()> {
        let now = Instant::now();
        if self.exhausted_at(now) {
            bail!(
                "{} did not finish within {} seconds and was abandoned",
                self.what,
                self.limit.as_secs()
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Available {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

pub enum Outcome {
    UpToDate { latest: String },
    Available(Available),
}

#[derive(Deserialize)]
struct Manifest {
    #[serde(rename = "schemaVersion")]
    schema_version: u32,
    version: String,
    artifacts: Vec<Artifact>,
}

#[derive(Deserialize)]
struct Artifact {
    kind: String,
    architecture: String,
    url: String,
    sha256: String,
    #[serde(default)]
    size: u64,
}

fn validate_url(raw: &str, require_artifact_path: bool) -> Result<url::Url> {
    let u = url::Url::parse(raw).context("update URL is not a URL")?;
    if u.scheme() != "https" {
        bail!("update URL is not https");
    }
    if u.host_str() != Some(ALLOWED_HOST) {
        bail!("update URL host is not {ALLOWED_HOST}");
    }
    if !matches!(u.port(), None | Some(443)) {
        bail!("update URL uses a non-default port");
    }
    if !u.username().is_empty() || u.password().is_some() {
        bail!("update URL carries credentials");
    }
    if u.query().is_some() || u.fragment().is_some() {
        bail!("update URL carries a query or fragment");
    }
    if require_artifact_path && !u.path().starts_with(ARTIFACT_PREFIX) {
        bail!("artifact URL is outside {ARTIFACT_PREFIX}");
    }
    Ok(u)
}

fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the updater's runtime")
}

fn base_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(TIMEOUT)
        .read_timeout(TIMEOUT)
        .min_tls_version(reqwest::tls::Version::TLS_1_3)
        .https_only(true)
        .user_agent(concat!("aitierlist/", env!("CARGO_PKG_VERSION")))
}

fn client() -> Result<reqwest::Client> {
    base_builder().build().context("building https client")
}

fn client_h3() -> Result<reqwest::Client> {
    base_builder()
        .http3_prior_knowledge()
        .build()
        .context("building http/3 client")
}

fn transport_name(version: reqwest::Version) -> &'static str {
    match version {
        reqwest::Version::HTTP_3 => "HTTP/3 (QUIC)",
        reqwest::Version::HTTP_2 => "HTTP/2",
        reqwest::Version::HTTP_11 | reqwest::Version::HTTP_10 => "HTTP/1.1",
        _ => "an unknown protocol",
    }
}

fn is_transport_failure(e: &anyhow::Error) -> bool {
    e.downcast_ref::<reqwest::Error>().is_some_and(|r| {
        r.is_connect() || r.is_timeout() || r.is_request() || r.is_body() || r.is_decode()
    })
}

struct Fetched {
    response: reqwest::Response,
    transport: &'static str,
}

async fn get_with(
    c: &reqwest::Client,
    url: &str,
    require_artifact_path: bool,
    budget: &Budget,
) -> Result<Fetched> {
    let mut current = validate_url(url, require_artifact_path)?;
    for _ in 0..=MAX_REDIRECTS {
        budget.check()?;
        let resp = c
            .get(current.clone())
            .timeout(budget.remaining_at(Instant::now()))
            .send()
            .await
            .context("request failed")?;
        if !resp.status().is_redirection() {
            if !resp.status().is_success() {
                bail!("server returned {}", resp.status());
            }
            let transport = transport_name(resp.version());
            return Ok(Fetched {
                response: resp,
                transport,
            });
        }
        let loc = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .context("redirect without a Location")?;
        let next = current.join(loc).context("bad redirect target")?;
        current = validate_url(next.as_str(), require_artifact_path)?;
    }
    bail!("too many redirects")
}

async fn attempt_bounded(
    c: &reqwest::Client,
    url: &str,
    require_artifact_path: bool,
    limit: u64,
    budget: &Budget,
) -> Result<(Vec<u8>, &'static str)> {
    let fetched = get_with(c, url, require_artifact_path, budget).await?;
    let transport = fetched.transport;
    let mut resp = fetched.response;
    if let Some(len) = resp.content_length()
        && len > limit
    {
        bail!("response is larger than the {limit}-byte limit");
    }
    let mut buf = Vec::new();
    loop {
        budget.check()?;
        let Some(chunk) = resp.chunk().await? else {
            break;
        };
        if buf.len() as u64 + chunk.len() as u64 > limit {
            bail!("response exceeded the {limit}-byte limit");
        }
        buf.extend_from_slice(&chunk);
        budget.check()?;
    }
    Ok((buf, transport))
}

async fn fetch_bounded(
    url: &str,
    require_artifact_path: bool,
    limit: u64,
    budget: &Budget,
) -> Result<(Vec<u8>, &'static str)> {
    budget.check()?;
    match attempt_bounded(&client_h3()?, url, require_artifact_path, limit, budget).await {
        Err(e) if is_transport_failure(&e) => {
            budget.check()?;
            attempt_bounded(&client()?, url, require_artifact_path, limit, budget).await
        }
        other => other,
    }
}

fn get_bounded(
    url: &str,
    require_artifact_path: bool,
    limit: u64,
    budget: &Budget,
) -> Result<(Vec<u8>, &'static str)> {
    runtime()?.block_on(fetch_bounded(url, require_artifact_path, limit, budget))
}

pub struct Checked {
    pub outcome: Outcome,
    pub transport: &'static str,
}

pub fn check(current_version: &str) -> Result<Checked> {
    let outcome = check_direct(current_version);
    match &outcome {
        Ok(checked) => match &checked.outcome {
            Outcome::Available(av) => record_check(Some(&av.version)),
            Outcome::UpToDate { latest } => record_check(Some(latest)),
        },
        Err(_) => record_check(None),
    }
    outcome
}

fn check_direct(current_version: &str) -> Result<Checked> {
    let budget = Budget::new(CHECK_BUDGET, "the update check");
    let (manifest_bytes, transport) =
        get_bounded(MANIFEST_URL, false, MAX_MANIFEST_BYTES, &budget)?;
    let manifest: Manifest =
        serde_json::from_slice(&manifest_bytes).context("manifest is not valid JSON")?;
    if manifest.schema_version != 1 {
        bail!("unsupported manifest schema {}", manifest.schema_version);
    }

    if !is_newer(&manifest.version, current_version)? {
        return Ok(Checked {
            outcome: Outcome::UpToDate {
                latest: manifest.version,
            },
            transport,
        });
    }

    let want_arch = "x64";
    let want_kind = "portable";
    let art = manifest
        .artifacts
        .iter()
        .find(|a| a.kind == want_kind && a.architecture == want_arch)
        .with_context(|| format!("manifest has no {want_kind}/{want_arch} artifact"))?;

    let expected_name = format!(
        "aitierlist-{}-{want_kind}-{want_arch}.exe",
        manifest.version
    );
    let expected_path = format!("{ARTIFACT_PREFIX}{}/{expected_name}", manifest.version);
    let u = validate_url(&art.url, true)?;
    if u.path() != expected_path {
        bail!("artifact path is not the expected {expected_path}");
    }
    if art.sha256.len() != 64 || !art.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("artifact sha256 is malformed");
    }
    if art.size > MAX_ARTIFACT_BYTES {
        bail!("artifact is larger than the {MAX_ARTIFACT_BYTES}-byte limit");
    }

    Ok(Checked {
        outcome: Outcome::Available(Available {
            version: manifest.version.clone(),
            url: art.url.clone(),
            sha256: art.sha256.to_lowercase(),
            size: art.size,
        }),
        transport,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReleaseVersion {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Option<Vec<PreIdent>>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum PreIdent {
    Numeric(u64),
    Text(String),
}

pub(crate) fn parse_release_version(text: &str) -> Option<ReleaseVersion> {
    if !(1..=32).contains(&text.len()) {
        return None;
    }
    let (numeric, pre) = match text.split_once('-') {
        None => (text, None),
        Some((_, "")) => return None,
        Some((numeric, pre)) => (numeric, Some(pre)),
    };
    let mut parts = numeric.split('.');
    let major = parse_numeric_part(parts.next()?)?;
    let minor = parse_numeric_part(parts.next()?)?;
    let patch = parse_numeric_part(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    let pre = match pre {
        None => None,
        Some(pre) => Some(
            pre.split('.')
                .map(parse_ident)
                .collect::<Option<Vec<_>>>()?,
        ),
    };
    Some(ReleaseVersion {
        major,
        minor,
        patch,
        pre,
    })
}

fn parse_numeric_part(part: &str) -> Option<u64> {
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

fn parse_ident(ident: &str) -> Option<PreIdent> {
    if ident.is_empty() || !ident.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return None;
    }
    if ident.bytes().all(|byte| byte.is_ascii_digit()) {
        match ident.parse() {
            Ok(n) => Some(PreIdent::Numeric(n)),
            Err(_) => Some(PreIdent::Text(ident.to_owned())),
        }
    } else {
        Some(PreIdent::Text(ident.to_owned()))
    }
}

impl PartialOrd for ReleaseVersion {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ReleaseVersion {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.major
            .cmp(&other.major)
            .then(self.minor.cmp(&other.minor))
            .then(self.patch.cmp(&other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

fn is_newer(candidate: &str, current: &str) -> Result<bool> {
    let candidate =
        parse_release_version(candidate).context("version is not a published version")?;
    let current = parse_release_version(current).context("version is not a published version")?;
    Ok(candidate > current)
}

pub fn data_dir() -> Result<PathBuf> {
    let base = directories::BaseDirs::new()
        .ok_or_else(|| anyhow::anyhow!("could not locate user directories"))?;
    let dir = base.data_local_dir().join("aitierlist");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn stamp_path() -> Option<PathBuf> {
    data_dir().ok().map(|dir| dir.join("last-update-check"))
}

pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

pub fn due_for_check() -> bool {
    let Some(p) = stamp_path() else { return false };
    match std::fs::metadata(&p).and_then(|m| m.modified()) {
        Ok(t) => t.elapsed().map(|e| e >= CHECK_INTERVAL).unwrap_or(true),
        Err(_) => true,
    }
}

pub fn cached_published_version() -> Option<String> {
    let text = std::fs::read_to_string(stamp_path()?).ok()?;
    let version = text.trim();
    parse_release_version(version)?;
    Some(version.to_string())
}

pub fn record_check(published_version: Option<&str>) {
    let Some(p) = stamp_path() else { return };
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let retained = published_version
        .map(str::as_bytes)
        .map(Vec::from)
        .or_else(|| std::fs::read(&p).ok())
        .unwrap_or_default();
    let _ = crate::file_tx::replace_file_contents(&p, &retained);
}

use crate::handoff::{self, StageHandoff};

pub struct StagedUpdate {
    stage: crate::file_tx::OwnedStagingFile,
    version: String,
    size: u64,
    sha256: String,
}

impl StagedUpdate {
    fn authenticate(&mut self) -> Result<()> {
        let identity = self.stage.identity();
        let file = self.stage.file()?;
        file.seek(std::io::SeekFrom::Start(0))
            .context("rewinding the staged update")?;
        let (measured, digest) =
            crate::file_tx::hash_reader(file, "hashed update size overflowed u64")
                .context("reading the staged update")?;
        let digest = hex::encode(digest);
        if measured != self.size || digest != self.sha256 {
            bail!(
                "the downloaded update changed on disk before it could be installed \
                 (expected {} bytes / {}, found {measured} bytes / {digest})",
                self.size,
                self.sha256
            );
        }
        if crate::file_tx::identity_of(file)? != identity {
            bail!("the downloaded update is no longer the file that was verified");
        }
        Ok(())
    }
}

pub fn download(av: &Available, mut progress: impl FnMut(u64, u64)) -> Result<StagedUpdate> {
    let budget = Budget::new(ARTIFACT_BUDGET, "the update download");
    let exe = std::env::current_exe().context("locating the running executable")?;
    let mut stage = crate::file_tx::OwnedStagingFile::create_beside(&exe)?;

    let done = runtime()?.block_on(async {
        match attempt_artifact(&client_h3()?, av, &budget, &mut stage, &mut progress).await {
            Err(e) if is_transport_failure(&e) => {
                budget.check()?;
                attempt_artifact(&client()?, av, &budget, &mut stage, &mut progress).await
            }
            other => other,
        }
    })?;

    if av.size != 0 && done != av.size {
        bail!("size mismatch (expected {}, got {done})", av.size);
    }

    let mut staged = StagedUpdate {
        stage,
        version: av.version.clone(),
        size: done,
        sha256: av.sha256.clone(),
    };
    staged
        .authenticate()
        .context("the downloaded update did not match the published SHA-256")?;
    Ok(staged)
}

async fn attempt_artifact(
    c: &reqwest::Client,
    av: &Available,
    budget: &Budget,
    stage: &mut crate::file_tx::OwnedStagingFile,
    progress: &mut impl FnMut(u64, u64),
) -> Result<u64> {
    use std::io::{Seek, Write};

    budget.check()?;
    {
        let file = stage.file()?;
        file.set_len(0).context("truncating the staged update")?;
        file.seek(std::io::SeekFrom::Start(0))
            .context("rewinding the staged update")?;
    }

    let fetched = get_with(c, &av.url, true, budget).await?;
    let mut resp = fetched.response;
    let total = resp.content_length().unwrap_or(av.size);
    if total > MAX_ARTIFACT_BYTES {
        bail!("artifact is implausibly large");
    }

    let mut done: u64 = 0;
    let file = stage.file()?;
    loop {
        budget.check()?;
        let Some(chunk) = resp.chunk().await? else {
            break;
        };
        budget.check()?;
        done += chunk.len() as u64;
        if done > MAX_ARTIFACT_BYTES {
            bail!("artifact exceeded the size limit mid-download");
        }
        file.write_all(&chunk)?;
        budget.check()?;
        progress(done, total);
        budget.check()?;
    }
    file.flush().context("flushing the staged update")?;
    budget.check()?;
    file.sync_all().context("syncing the staged update")?;
    budget.check()?;
    Ok(done)
}

pub fn hand_off(staged: &mut StagedUpdate, relaunch: bool) -> Result<()> {
    handoff::hand_off(
        StageHandoff {
            version: &staged.version,
            size: staged.size,
            sha256: &staged.sha256,
            stage: &mut staged.stage,
        },
        relaunch,
    )
}

pub fn install(staged: &mut StagedUpdate) -> Result<std::convert::Infallible> {
    hand_off(staged, true)?;
    std::process::exit(0);
}

pub fn cleanup_previous_update() {
    handoff::cleanup_previous_update();
}

pub fn last_handoff_status() -> Option<handoff::Status> {
    let directory = handoff::state_dir().ok()?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&directory).ok()?.flatten() {
        let path = entry.path();
        let is_status = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("handoff-") && n.ends_with(".status.json"));
        if !is_status {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if newest.as_ref().is_none_or(|(at, _)| modified > *at) {
            newest = Some((modified, path));
        }
    }
    let (_, path) = newest?;
    let text = std::fs::read_to_string(&path).ok()?;
    let status = serde_json::from_str::<handoff::Status>(&text).ok()?;
    let _ = std::fs::remove_file(&path);
    Some(status)
}
