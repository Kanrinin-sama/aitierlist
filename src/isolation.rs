use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agent_setup::HostKind;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    pub kind: HostKind,
    pub id: String,
    pub binding_id: String,
    pub account_id: String,
    pub executable: PathBuf,
    pub model: String,
    pub effort: Option<String>,
    pub workspace: PathBuf,
    pub collaboration_root: PathBuf,
    pub home: PathBuf,
    pub authenticated: bool,
    pub executable_identity: String,
    pub permission: String,
    pub network_scope: Vec<String>,
    pub source_scope: Vec<String>,
    pub exclusions: Vec<String>,
}

pub fn launch_identity(route: &Route) -> Result<String> {
    let arguments = worker_arguments(route, Path::new("__BRIEF__"))?;
    let bytes = serde_json::to_vec(&serde_json::json!({
        "policy_version": crate::team_policy::VERSION,
        "kind": route.kind,
        "binding_id": route.binding_id,
        "account_id": route.account_id,
        "executable_identity": route.executable_identity,
        "model": route.model,
        "effort": route.effort,
        "workspace": route.workspace,
        "collaboration_root": route.collaboration_root,
        "home": route.home,
        "arguments": arguments,
        "permission": route.permission,
        "network_scope": route.network_scope,
        "source_scope": route.source_scope,
        "exclusions": route.exclusions,
    }))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fleet {
    pub version: u32,
    pub policy_version: String,
    pub profile_identity: String,
    pub directory: PathBuf,
    pub ledger: PathBuf,
    pub routes: Vec<Route>,
    pub conductors: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct QueuedJob {
    route: Route,
    job: crate::team_policy::TaskRequest,
    ledger_job_id: String,
    policy_identity: String,
    profile_identity: String,
    horizon_id: String,
}

#[derive(Serialize, Deserialize)]
struct JobResult {
    task_id: String,
    state: String,
    exit_code: Option<i32>,
    message: String,
    output_excerpt: String,
}

#[derive(Serialize, Deserialize)]
struct ControllerIdentity {
    pid: u32,
    created: u64,
}

#[cfg(windows)]
fn process_created(handle: windows_sys::Win32::Foundation::HANDLE) -> Result<u64> {
    use windows_sys::Win32::Foundation::FILETIME;
    let mut created: FILETIME = unsafe { std::mem::zeroed() };
    let mut exited = created;
    let mut kernel = created;
    let mut user = created;
    ensure!(
        unsafe {
            windows_sys::Win32::System::Threading::GetProcessTimes(
                handle,
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        } != 0,
        "Cannot identify runtime controller process"
    );
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

impl ControllerIdentity {
    fn current() -> Result<Self> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId};
            let handle = unsafe { GetCurrentProcess() };
            Ok(Self {
                pid: unsafe { GetCurrentProcessId() },
                created: process_created(handle)?,
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                pid: std::process::id(),
                created: 0,
            })
        }
    }
    fn child(child: &std::process::Child) -> Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            Ok(Self {
                pid: child.id(),
                created: process_created(child.as_raw_handle() as _)?,
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                pid: child.id(),
                created: 0,
            })
        }
    }
    fn alive(&self) -> bool {
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
            };
            let handle =
                unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | 0x00100000, 0, self.pid) };
            if handle.is_null() {
                return false;
            }
            let same = process_created(handle).is_ok_and(|created| created == self.created)
                && unsafe { WaitForSingleObject(handle, 0) }
                    == windows_sys::Win32::Foundation::WAIT_TIMEOUT;
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(handle);
            }
            same
        }
        #[cfg(not(windows))]
        {
            false
        }
    }
}

fn probe_output(path: &Path, arguments: &[&str]) -> std::io::Result<std::process::Output> {
    let mut command = Command::new(path);
    command.args(arguments);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.output()
}

pub fn capability(kind: &HostKind, executable: Option<&Path>) -> (bool, String) {
    if !matches!(
        kind,
        HostKind::Claude
            | HostKind::Codex
            | HostKind::Muse
            | HostKind::Grok
            | HostKind::Antigravity
    ) {
        return (false, "No verified control disables every user and project customization source for this adapter.".to_owned());
    }
    let supported = executable
        .and_then(|path| {
            let arguments: &[&str] = if matches!(kind, HostKind::Codex | HostKind::Muse) {
                &["exec", "--help"]
            } else {
                &["--help"]
            };
            probe_output(path, arguments).ok()
        })
        .is_some_and(|output| {
            let help = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let required: &[&str] = match kind {
                HostKind::Claude => &["--safe-mode", "--model", "--effort"],
                HostKind::Codex => &[
                    "--ignore-user-config",
                    "--ignore-rules",
                    "--model",
                    "--json",
                    "--output-last-message",
                ],
                HostKind::Muse => &[
                    "--prompt-file",
                    "--model",
                    "--reasoning-effort",
                    "--workspace",
                    "--json",
                ],
                HostKind::Grok => &[
                    "--prompt-file",
                    "--model",
                    "--reasoning-effort",
                    "--output-format",
                    "--no-subagents",
                ],
                HostKind::Antigravity => &[
                    "--print",
                    "--prompt-interactive",
                    "--model",
                    "--effort",
                    "--output-format",
                    "--print-timeout",
                ],
                _ => &[],
            };
            output.status.success() && required.iter().all(|flag| help.contains(flag))
        });
    (
        supported,
        if supported {
            "Required native flags verified. Isolation uses a clean profile and independent sanitized project copy; managed security policy remains. Fresh sign-in may be required.".to_owned()
        } else {
            "This executable does not advertise all required native isolation/dispatch controls."
                .to_owned()
        },
    )
}

pub fn executable_identity(path: &Path) -> Result<String> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    Ok(format!("{}:{modified}", metadata.len()))
}

pub fn executable_version(path: &Path) -> Result<String> {
    let output = probe_output(path, &["--version"])?;
    ensure!(
        output.status.success(),
        "Cannot establish native executable version"
    );
    let version = String::from_utf8(output.stdout)?.trim().to_owned();
    ensure!(
        !version.is_empty() && version.len() <= 2048,
        "Native version output is unavailable"
    );
    Ok(version)
}

pub fn conductor_route(manifest: &Path, host_id: &str) -> Result<Option<Route>> {
    let fleet = load_fleet(manifest)?;
    Ok(fleet
        .conductors
        .get(host_id)
        .and_then(|id| {
            fleet
                .routes
                .iter()
                .find(|route| route.id == *id && route.authenticated)
        })
        .cloned())
}

pub struct ConductorReservation {
    profile_identity: String,
    job_id: String,
}

pub fn reserve_conductor_launch(manifest: &Path, route: &Route) -> Result<ConductorReservation> {
    let fleet = load_fleet(manifest)?;
    let paths = crate::agent_setup::paths()?;
    let (profile, ledger, profile_identity) = crate::agent_setup::read_runtime(&paths)?;
    ensure!(
        profile_identity == fleet.profile_identity,
        "Conductor capability profile is stale"
    );
    ensure!(
        ledger
            .jobs
            .iter()
            .filter(|job| {
                job.seat == Some(crate::types::Seat::Orchestrator)
                    && job.status == crate::agent_setup::JobStatus::Running
            })
            .all(|job| job.binding_id == route.binding_id),
        "Every active project conductor must use the fixed running binding"
    );
    let coefficients: Vec<_> = profile
        .native_coefficients
        .iter()
        .filter(|coefficient| {
            coefficient.binding_id == route.binding_id
                && coefficient.seat == crate::types::Seat::Orchestrator
                && coefficient.workflow == crate::portfolio::WorkClass::Focused
        })
        .cloned()
        .collect();
    ensure!(
        !coefficients.is_empty(),
        "Conductor startup/resume has no native reserve coefficient"
    );
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let job_id = format!("conductor-session-{stamp}");
    crate::agent_setup::transact(
        &paths,
        &profile_identity,
        Some(ledger.revision),
        |_, current| {
            for coefficient in &coefficients {
                let window = current
                    .accounts
                    .iter()
                    .find(|account| account.account_id == coefficient.account_id)
                    .and_then(|account| {
                        account
                            .windows
                            .iter()
                            .find(|window| window.window_id == coefficient.window_id)
                    })
                    .context("Conductor native window is unavailable")?;
                let remaining = window
                    .snapshot
                    .value
                    .as_ref()
                    .context("Conductor meter snapshot is unavailable")?
                    .remaining
                    - window.spent_since_snapshot
                    - current
                        .jobs
                        .iter()
                        .flat_map(|job| &job.holds)
                        .filter(|hold| {
                            hold.account_id == coefficient.account_id
                                && hold.window_id == coefficient.window_id
                        })
                        .map(|hold| hold.amount)
                        .sum::<f64>();
                ensure!(
                    remaining + 1e-9 >= coefficient.reserve,
                    "Conductor startup/resume does not fit native quota"
                );
            }
            current.jobs.push(crate::agent_setup::JobLedger {
                id: job_id.clone(),
                idempotency_key: job_id.clone(),
                binding_id: route.binding_id.clone(),
                route_id: route.id.clone(),
                status: crate::agent_setup::JobStatus::Held,
                dependencies: Vec::new(),
                attempt_ids: Vec::new(),
                worker_handle: unknown_fact(),
                artifacts: Vec::new(),
                artifact_records: Vec::new(),
                recovery: unknown_fact(),
                holds: coefficients
                    .iter()
                    .map(|coefficient| native_hold(current, &job_id, coefficient))
                    .collect::<Result<Vec<_>>>()?,
                project_id: "conductor".to_owned(),
                parent_task_id: job_id.clone(),
                workflow: Some(crate::portfolio::WorkClass::Focused),
                seat: Some(crate::types::Seat::Orchestrator),
                risk: None,
                evidence_requirements: Vec::new(),
                conditional: false,
                actual_settled: Vec::new(),
                outcome: None,
                scheduled_start: None,
                scheduled_end: None,
                remaining_seconds: Some(
                    coefficients
                        .iter()
                        .map(|coefficient| coefficient.reserve_seconds)
                        .fold(0.0, f64::max),
                ),
                settlement_keys: Vec::new(),
                settlement_digests: std::collections::BTreeMap::new(),
                started_at: None,
                ended_at: None,
                input_context_bucket: None,
                tool_calls: std::collections::BTreeMap::new(),
            });
            Ok(())
        },
    )?;
    Ok(ConductorReservation {
        profile_identity,
        job_id,
    })
}

pub fn conductor_started(
    reservation: ConductorReservation,
    child: &std::process::Child,
) -> Result<()> {
    let identity = ControllerIdentity::child(child)?;
    let paths = crate::agent_setup::paths()?;
    crate::agent_setup::transact(&paths, &reservation.profile_identity, None, |_, ledger| {
        let job = ledger
            .jobs
            .iter_mut()
            .find(|job| job.id == reservation.job_id)
            .context("Conductor reservation is absent")?;
        ensure!(
            job.status == crate::agent_setup::JobStatus::Held,
            "Conductor reservation is not launchable"
        );
        job.status = crate::agent_setup::JobStatus::Running;
        job.attempt_ids
            .push(format!("attempt-{}-{}", job.id, identity.created));
        job.worker_handle = crate::agent_setup::Fact {
            status: crate::agent_setup::FactStatus::Verified,
            value: Some(format!("{}:{}", identity.pid, identity.created)),
            evidence: vec!["launcher process identity".to_owned()],
        };
        job.started_at = Some(
            time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)?,
        );
        Ok(())
    })
}

pub fn conductor_launch_failed(reservation: ConductorReservation) -> Result<()> {
    let paths = crate::agent_setup::paths()?;
    crate::agent_setup::transact(&paths, &reservation.profile_identity, None, |_, ledger| {
        let job = ledger
            .jobs
            .iter_mut()
            .find(|job| job.id == reservation.job_id)
            .context("Conductor reservation is absent")?;
        ensure!(
            job.status == crate::agent_setup::JobStatus::Held && job.attempt_ids.is_empty(),
            "Only a never-started conductor reservation can be released"
        );
        job.status = crate::agent_setup::JobStatus::Released;
        job.holds.clear();
        Ok(())
    })
}

pub fn clean_command(executable: &Path, home: &Path, workspace: &Path) -> Command {
    let mut command = Command::new(executable);
    command.env_clear();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    for name in [
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATH",
        "PATHEXT",
        "TEMP",
        "TMP",
        "TERM",
        "COLORTERM",
        "LANG",
        "LC_ALL",
        "COMPUTERNAME",
        "HOSTNAME",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("APPDATA", home.join("AppData/Roaming"))
        .env("LOCALAPPDATA", home.join("AppData/Local"))
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("CODEX_HOME", home.join(".codex"))
        .env("GROK_HOME", home.join(".grok"))
        .current_dir(workspace);
    if let Some(generation) = home.parent().and_then(Path::parent) {
        let mut paths = vec![generation.join("bin")];
        if let Some(existing) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&existing));
        }
        if let Ok(value) = std::env::join_paths(paths) {
            command.env("PATH", value);
        }
    }
    command
}

pub fn prepare_home(home: &Path, generation: &Path) -> Result<()> {
    crate::host_install::destination_allowed(home, &generation.join("homes"))?;
    for relative in [
        "",
        ".codex",
        ".claude",
        ".grok",
        ".gemini",
        ".config/muse",
        "AppData/Local",
        "AppData/Roaming",
    ] {
        let directory = home.join(relative);
        crate::host_install::destination_allowed(&directory, home)?;
        fs::create_dir_all(directory)?;
    }
    Ok(())
}

pub fn bootstrap(workspace: &Path, home: &Path, git: &Path) -> Result<()> {
    fs::create_dir_all(workspace)?;
    if workspace.join(".git").is_dir() {
        return Ok(());
    }
    let template = home.join("empty-git-template");
    fs::create_dir_all(&template)?;
    let status = clean_command(git, home, workspace)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args(["init", "--quiet", "--template"])
        .arg(template)
        .arg(workspace)
        .status()?;
    ensure!(
        status.success(),
        "Cannot initialize the independent bootstrap workspace"
    );
    Ok(())
}

pub fn conductor_arguments(
    kind: &HostKind,
    workspace: &Path,
    app_directory: &Path,
    prompt: String,
) -> Result<Vec<String>> {
    let mut arguments = match kind {
        HostKind::Claude => vec!["--safe-mode".to_owned()],
        HostKind::Codex => vec![
            "-c".to_owned(),
            "project_doc_max_bytes=0".to_owned(),
            "-c".to_owned(),
            "default_permissions=\"aitierlist\"".to_owned(),
            "-c".to_owned(),
            "permissions.aitierlist.extends=\":workspace\"".to_owned(),
            "-c".to_owned(),
            format!(
                "permissions.aitierlist.workspace_roots={{{}=true}}",
                serde_json::to_string(&app_directory.to_string_lossy())?
            ),
            "--ignore-user-config".to_owned(),
            "--ignore-rules".to_owned(),
            "--disable".to_owned(),
            "multi_agent".to_owned(),
        ],
        HostKind::Muse => vec![
            "--workspace".to_owned(),
            workspace.to_string_lossy().into_owned(),
        ],
        HostKind::Grok => vec!["--no-subagents".to_owned()],
        HostKind::Antigravity => vec!["--prompt-interactive".to_owned()],
        _ => anyhow::bail!("Unsupported isolated conductor"),
    };
    arguments.push(prompt);
    Ok(arguments)
}

pub fn mapped_effort(kind: &HostKind, arguments: &[String]) -> Result<Option<String>> {
    if arguments.is_empty() {
        return Ok(None);
    }
    ensure!(
        arguments.len() == 2,
        "Effort mapping must contain one native option and value"
    );
    let value = match kind {
        HostKind::Codex => {
            ensure!(
                matches!(arguments[0].as_str(), "-c" | "--config"),
                "Codex effort mapping must use the reasoning configuration key"
            );
            let raw = arguments[1]
                .strip_prefix("model_reasoning_effort=")
                .context("Unsupported Codex effort configuration")?;
            raw.trim_matches('"').to_owned()
        }
        HostKind::Claude | HostKind::Antigravity => {
            ensure!(
                arguments[0] == "--effort",
                "Unsupported native effort option"
            );
            arguments[1].clone()
        }
        HostKind::Muse | HostKind::Grok => {
            ensure!(
                matches!(arguments[0].as_str(), "--reasoning-effort" | "--effort"),
                "Unsupported native effort option"
            );
            arguments[1].clone()
        }
        _ => anyhow::bail!("Unsupported isolated effort mapping"),
    };
    let supported = match kind {
        HostKind::Claude => matches!(value.as_str(), "low" | "medium" | "high" | "xhigh" | "max"),
        HostKind::Antigravity => matches!(value.as_str(), "low" | "medium" | "high"),
        HostKind::Codex | HostKind::Muse => matches!(
            value.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "ultra"
        ),
        HostKind::Grok => matches!(
            value.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "deep"
        ),
        _ => false,
    };
    ensure!(
        supported,
        "Unsupported effort value for this native adapter"
    );
    Ok(Some(value))
}

fn slug(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(1_048_577)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1_048_576, "Runtime input exceeds 1 MiB");
    Ok(serde_json::from_slice(&bytes)?)
}

fn load_fleet(path: &Path) -> Result<Fleet> {
    let mut fleet: Fleet = read_json(path)?;
    ensure!(
        fleet.version == 1 && fleet.directory.is_absolute(),
        "Unsupported fleet manifest"
    );
    let active = fleet.directory.join("active-fleet.json");
    if active.is_file() {
        let filename: String = read_json(&active)?;
        ensure!(
            filename.starts_with("fleet-")
                && filename.ends_with(".json")
                && !filename.contains(['/', '\\']),
            "Invalid active fleet reference"
        );
        let compiled: Fleet = read_json(&fleet.directory.join(filename))?;
        ensure!(
            compiled.version == fleet.version
                && compiled.directory == fleet.directory
                && compiled.ledger == fleet.ledger,
            "Active fleet belongs to a different generation"
        );
        fleet = compiled;
    }
    ensure!(
        path.canonicalize()?.parent() == Some(fleet.directory.canonicalize()?.as_path()),
        "Fleet manifest does not belong to this generation"
    );
    Ok(fleet)
}

fn write_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    Ok(())
}

fn unknown_fact<T>() -> crate::agent_setup::Fact<T> {
    crate::agent_setup::Fact {
        status: crate::agent_setup::FactStatus::Unknown,
        value: None,
        evidence: Vec::new(),
    }
}

fn native_hold(
    ledger: &crate::agent_setup::Ledger,
    visit_id: &str,
    coefficient: &crate::agent_setup::NativeCoefficient,
) -> Result<crate::agent_setup::Hold> {
    let window = ledger
        .accounts
        .iter()
        .find(|account| account.account_id == coefficient.account_id)
        .and_then(|account| {
            account
                .windows
                .iter()
                .find(|window| window.window_id == coefficient.window_id)
        })
        .context("Native hold window is absent")?;
    Ok(crate::agent_setup::Hold {
        id: format!("hold-{visit_id}-{}", coefficient.window_id),
        account_id: coefficient.account_id.clone(),
        window_id: coefficient.window_id.clone(),
        amount: coefficient.reserve,
        expected: coefficient.expected,
        coefficient_id: coefficient.id.clone(),
        reflected_through: None,
        window_start: window.window_start.value.clone(),
        reset_at: window.reset_at.value.clone(),
    })
}

fn horizon_id(
    profile_identity: &str,
    revision: u64,
    tasks: &[crate::team_policy::TaskRequest],
) -> Result<String> {
    let bytes = serde_json::to_vec(&(
        crate::team_policy::VERSION,
        profile_identity,
        revision,
        tasks,
    ))?;
    Ok(format!(
        "horizon-{}",
        &hex::encode(Sha256::digest(bytes))[..24]
    ))
}

fn commit_horizon_once(
    fleet: &Fleet,
    new_task: Option<crate::team_policy::TaskRequest>,
) -> Result<crate::native_scheduler::NativeHorizon> {
    let setup_paths = crate::agent_setup::paths()?;
    ensure!(
        setup_paths.ledger.canonicalize()? == fleet.ledger.canonicalize()?,
        "Fleet ledger path changed"
    );
    let (profile, ledger, profile_identity) = crate::agent_setup::read_runtime(&setup_paths)?;
    ensure!(
        profile_identity == fleet.profile_identity
            && fleet.policy_version == crate::team_policy::VERSION,
        "Fleet policy or capability profile is stale; finalize again"
    );
    let generation_id = fleet
        .directory
        .file_name()
        .and_then(|name| name.to_str())
        .context("Generation identity is unavailable")?
        .to_owned();
    let snapshot = crate::snapshot::load(&fleet.directory)?;
    let mut all_tasks = ledger.tasks.clone();
    if let Some(task) = new_task {
        ensure!(
            task.generation_id == generation_id && task.workspace_id == snapshot.id,
            "Task belongs to another generation or workspace"
        );
        if let Some(existing) = all_tasks.iter().find(|existing| {
            existing.task_id == task.task_id || existing.idempotency_key == task.idempotency_key
        }) {
            ensure!(
                serde_json::to_value(existing)? == serde_json::to_value(&task)?,
                "Task identity is already bound to different content"
            );
        } else {
            all_tasks.push(task);
        }
    }
    let tasks: Vec<_> = all_tasks
        .iter()
        .filter(|task| task.generation_id == generation_id && task.workspace_id == snapshot.id)
        .cloned()
        .collect();
    let id = horizon_id(&profile_identity, ledger.revision, &tasks)?;
    let bindings: BTreeSet<_> = fleet
        .conductors
        .values()
        .filter_map(|id| {
            fleet
                .routes
                .iter()
                .find(|route| route.id == *id)
                .map(|route| route.binding_id.clone())
        })
        .collect();
    ensure!(
        bindings.len() <= 1,
        "Fleet conductors use different bindings"
    );
    let exported_conductor = bindings.into_iter().next();
    let mut horizon = crate::native_scheduler::allocate(
        &tasks,
        &fleet.routes,
        &profile,
        &ledger,
        exported_conductor.as_deref(),
        &id,
        4096,
    )?;
    horizon.ledger_revision = ledger.revision + 1;
    let immutable_artifact = fleet
        .directory
        .join("plans")
        .join(format!("{}.json", horizon.horizon_id));
    fs::create_dir_all(
        immutable_artifact
            .parent()
            .context("Plan directory missing")?,
    )?;
    write_new(&immutable_artifact, &horizon)?;
    let artifact = fleet.directory.join("native-plan.v1.json");
    let markdown = fleet.directory.join("native-plan.v1.md");
    let artifact_bytes = serde_json::to_vec_pretty(&horizon)?;
    let markdown_bytes = crate::orchestration::render_native_horizon(&horizon)?.into_bytes();
    crate::agent_setup::transact(
        &setup_paths,
        &profile_identity,
        Some(ledger.revision),
        |_, current| {
            let scoped_tasks: std::collections::BTreeSet<_> =
                tasks.iter().map(|task| task.task_id.as_str()).collect();
            current.jobs.retain(|job| {
                !scoped_tasks.contains(job.parent_task_id.as_str())
                    || job.status == crate::agent_setup::JobStatus::Running
                    || job.status == crate::agent_setup::JobStatus::Completed
                    || !job.attempt_ids.is_empty()
            });
            current.tasks = all_tasks.clone();
            for plan in &horizon.plans {
                for assignment in &plan.assignments {
                    if current.jobs.iter().any(|job| job.id == assignment.visit.id) {
                        continue;
                    }
                    let schedule = horizon
                        .scheduled_visits
                        .iter()
                        .find(|visit| visit.visit_id == assignment.visit.id);
                    current.jobs.push(crate::agent_setup::JobLedger {
                        id: assignment.visit.id.clone(),
                        idempotency_key: format!("{}-{}", plan.task_id, assignment.visit.id),
                        binding_id: assignment.binding_id.clone(),
                        route_id: assignment.route_id.clone(),
                        status: crate::agent_setup::JobStatus::Held,
                        dependencies: assignment.visit.dependencies.clone(),
                        attempt_ids: Vec::new(),
                        worker_handle: unknown_fact(),
                        artifacts: Vec::new(),
                        artifact_records: Vec::new(),
                        recovery: unknown_fact(),
                        holds: assignment
                            .coefficients
                            .iter()
                            .map(|coefficient| {
                                native_hold(current, &assignment.visit.id, coefficient)
                            })
                            .collect::<Result<Vec<_>>>()?,
                        project_id: plan.project_id.clone(),
                        parent_task_id: plan.task_id.clone(),
                        workflow: Some(plan.workflow),
                        seat: Some(assignment.visit.seat),
                        risk: current
                            .tasks
                            .iter()
                            .find(|task| task.task_id == plan.task_id)
                            .map(|task| task.risk.clone()),
                        evidence_requirements: current
                            .tasks
                            .iter()
                            .find(|task| task.task_id == plan.task_id)
                            .map(|task| task.evidence_requirements.clone())
                            .unwrap_or_default(),
                        conditional: assignment.visit.conditional,
                        actual_settled: Vec::new(),
                        outcome: None,
                        scheduled_start: schedule.map(|visit| visit.start.clone()),
                        scheduled_end: schedule.map(|visit| visit.end.clone()),
                        remaining_seconds: schedule.and_then(|visit| {
                            let start = time::OffsetDateTime::parse(
                                &visit.start,
                                &time::format_description::well_known::Rfc3339,
                            )
                            .ok()?;
                            let end = time::OffsetDateTime::parse(
                                &visit.end,
                                &time::format_description::well_known::Rfc3339,
                            )
                            .ok()?;
                            Some((end - start).as_seconds_f64())
                        }),
                        settlement_keys: Vec::new(),
                        settlement_digests: std::collections::BTreeMap::new(),
                        started_at: None,
                        ended_at: None,
                        input_context_bucket: None,
                        tool_calls: std::collections::BTreeMap::new(),
                    });
                }
            }
            let pointer = crate::agent_setup::HorizonPointer {
                id: horizon.horizon_id.clone(),
                policy_version: horizon.policy_version.clone(),
                profile_identity: profile_identity.clone(),
                committed_revision: ledger.revision + 1,
                artifact_path: immutable_artifact.to_string_lossy().into_owned(),
                generation_id: generation_id.clone(),
            };
            current
                .horizons
                .retain(|existing| existing.generation_id != generation_id);
            current.horizons.push(pointer.clone());
            current.current_horizon = Some(pointer);
            Ok(())
        },
    )?;
    crate::file_tx::replace_file_contents(&artifact, &artifact_bytes)?;
    crate::file_tx::replace_file_contents(&markdown, &markdown_bytes)?;
    Ok(horizon)
}

fn commit_horizon(
    fleet: &Fleet,
    new_task: Option<crate::team_policy::TaskRequest>,
) -> Result<crate::native_scheduler::NativeHorizon> {
    let mut conflict = None;
    for _ in 0..8 {
        match commit_horizon_once(fleet, new_task.clone()) {
            Ok(horizon) => return Ok(horizon),
            Err(error)
                if error.to_string().contains("Ledger changed")
                    || error.to_string().contains("shared ledger is busy") =>
            {
                conflict = Some(error)
            }
            Err(error) => return Err(error),
        }
    }
    Err(conflict.unwrap_or_else(|| anyhow::anyhow!("Native horizon commit conflict")))
}

fn visit_ready(entry: &crate::agent_setup::JobLedger, ledger: &crate::agent_setup::Ledger) -> bool {
    let accepted = |id: &str| {
        ledger
            .jobs
            .iter()
            .find(|job| job.id == id)
            .is_some_and(|job| {
                job.status == crate::agent_setup::JobStatus::Completed
                    && job
                        .outcome
                        .as_ref()
                        .is_some_and(|outcome| outcome.artifact_accepted)
            })
    };
    if entry.conditional && entry.seat == Some(crate::types::Seat::Debugger) {
        let debugger_running = ledger.jobs.iter().any(|job| {
            job.parent_task_id == entry.parent_task_id
                && job.id != entry.id
                && job.seat == Some(crate::types::Seat::Debugger)
                && job.status == crate::agent_setup::JobStatus::Running
        });
        let initial_check_running = ledger.jobs.iter().any(|job| {
            job.parent_task_id == entry.parent_task_id
                && job.id.ends_with("-1")
                && matches!(
                    job.seat,
                    Some(crate::types::Seat::Reviewer | crate::types::Seat::Sanity)
                )
                && job.status == crate::agent_setup::JobStatus::Running
        });
        let triggered = ledger.jobs.iter().any(|job| {
            job.parent_task_id == entry.parent_task_id
                && job.id.ends_with("-1")
                && match job.seat {
                    Some(crate::types::Seat::Implementer) => {
                        job.status == crate::agent_setup::JobStatus::Blocked
                    }
                    Some(crate::types::Seat::Reviewer | crate::types::Seat::Sanity) => {
                        job.status == crate::agent_setup::JobStatus::Blocked
                            || (job.status == crate::agent_setup::JobStatus::Completed
                                && job
                                    .outcome
                                    .as_ref()
                                    .is_some_and(|outcome| !outcome.artifact_accepted))
                    }
                    _ => false,
                }
        });
        return triggered && !debugger_running && !initial_check_running;
    }
    entry
        .dependencies
        .iter()
        .all(|dependency| accepted(dependency))
}

pub fn dispatch(manifest: &Path, operation: &str, id: &str, request: Option<&Path>) -> Result<()> {
    let fleet = load_fleet(manifest)?;
    ensure!(slug(id), "Invalid route or task identifier");
    let jobs = fleet.directory.join("jobs");
    fs::create_dir_all(&jobs)?;
    match operation {
        "inspect" => {
            let table_file = fs::File::open(fleet.directory.join("table.json"))?;
            ensure!(
                table_file.metadata()?.len() <= 64 * 1024 * 1024,
                "Generation table exceeds 64 MiB"
            );
            let table: crate::types::Table = serde_json::from_reader(table_file)?;
            let seed: crate::host_install::DiscoverySeed =
                read_json(&fleet.directory.join("discovery.json"))?;
            let discovery = crate::host_install::refresh_discovery(&table, seed)?;
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({"generation_directory":fleet.directory,"computer_id":crate::agent_setup::computer_id(),"profile_path":discovery.paths.profile,"ledger_path":discovery.paths.ledger,"setup_message":discovery.setup.message,"required_bindings":crate::host_install::required_bindings(&table),"hosts":discovery.hosts.iter().filter(|host|host.installed).map(|host|serde_json::json!({"id":host.id,"kind":host.kind,"executable":host.executable,"executable_identity":host.isolation_identity,"executable_version":host.isolation_version,"isolation_supported":host.isolation_supported,"message":host.isolation_message})).collect::<Vec<_>>()})
                )?
            );
        }
        "finalize" => {
            let table_file = fs::File::open(fleet.directory.join("table.json"))?;
            ensure!(
                table_file.metadata()?.len() <= 64 * 1024 * 1024,
                "Generation table exceeds 64 MiB"
            );
            let table: crate::types::Table = serde_json::from_reader(table_file)?;
            let seed: crate::host_install::DiscoverySeed =
                read_json(&fleet.directory.join("discovery.json"))?;
            let discovery = crate::host_install::refresh_discovery(&table, seed)?;
            ensure!(
                discovery.setup.ready,
                "Complete the canonical onboarding profile and shared ledger first: {}",
                discovery.setup.message
            );
            let profile = discovery
                .setup
                .profile
                .as_ref()
                .context("Approved profile missing")?;
            let roots = profile
                .workflow
                .roots
                .value
                .as_ref()
                .context("Confirmed project root missing")?;
            ensure!(
                roots.len() == 1,
                "An isolated generation binds exactly one confirmed project root"
            );
            if !fleet.directory.join("snapshot.json").exists() {
                crate::snapshot::import(
                    Path::new(&roots[0]),
                    &fleet.directory,
                    &crate::host_install::git_binary()?,
                )?;
            }
            let compiled =
                crate::host_install::isolated_fleet(&discovery, &table, &fleet.directory)?;
            ensure!(
                !compiled.routes.is_empty(),
                "No approved isolated worker routes are available"
            );
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos();
            let filename = format!("fleet-{stamp}.json");
            write_new(&fleet.directory.join(&filename), &compiled)?;
            crate::file_tx::replace_file_contents(
                &fleet.directory.join("active-fleet.json"),
                &serde_json::to_vec(&filename)?,
            )?;
            println!(
                "Compiled {} fixed routes. Read the active fleet file before dispatch.",
                compiled.routes.len()
            );
        }
        "login" => {
            let (kind, executable, home, workspace, identity, account) =
                if let Some(route) = fleet.routes.iter().find(|route| route.id == id) {
                    (
                        route.kind,
                        route.executable.clone(),
                        route.home.clone(),
                        route.workspace.clone(),
                        route.executable_identity.clone(),
                        Some(route.account_id.clone()),
                    )
                } else {
                    let seed: crate::host_install::DiscoverySeed =
                        read_json(&fleet.directory.join("discovery.json"))?;
                    let host = seed
                        .hosts
                        .iter()
                        .find(|host| host.id == id && host.installed && host.isolation_supported)
                        .context("Unknown supported host or route ID; use fleet inspect setup")?;
                    (
                        host.kind,
                        host.executable
                            .clone()
                            .context("Captured host executable missing")?,
                        fleet.directory.join("homes").join(&host.id),
                        fleet.directory.join("bootstrap"),
                        host.isolation_identity
                            .clone()
                            .context("Captured executable identity missing")?,
                        None,
                    )
                };
            ensure!(
                executable.is_absolute() && executable_identity(&executable)? == identity,
                "Native executable changed; repeat discovery before login"
            );
            prepare_home(&home, &fleet.directory)?;
            if workspace == fleet.directory.join("bootstrap") {
                bootstrap(&workspace, &home, &crate::host_install::git_binary()?)?;
            }
            let login_arguments: &[&str] = match kind {
                HostKind::Claude => &["auth", "login"],
                HostKind::Codex | HostKind::Muse | HostKind::Grok => &["login"],
                HostKind::Antigravity => &[],
                _ => anyhow::bail!("Unsupported isolated login"),
            };
            let help = clean_command(&executable, &home, &workspace)
                .args(login_arguments)
                .arg("--help")
                .output()?;
            let help_text = format!(
                "{}{}",
                String::from_utf8_lossy(&help.stdout),
                String::from_utf8_lossy(&help.stderr)
            );
            ensure!(
                help.status.success()
                    && if matches!(kind, HostKind::Antigravity) {
                        help_text.contains("--prompt-interactive")
                    } else {
                        help_text.contains("login")
                    },
                "This native version does not advertise the required login entry point"
            );
            let mut command = clean_command(&executable, &home, &workspace);
            command.args(login_arguments);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x00000010);
            }
            command.spawn()?;
            println!(
                "Complete native sign-in, confirm {} for this generation in onboarding, then run fleet finalize setup.",
                account
                    .map(|id| format!("paid account {id}"))
                    .unwrap_or_else(|| "the signed-in paid account and host identity".to_owned())
            );
        }
        "diff" | "report" => {
            let snapshot = crate::snapshot::load(&fleet.directory)?;
            if let Some(route) = fleet.routes.first()
                && route.collaboration_root.canonicalize()? != snapshot.shadow.canonicalize()?
            {
                let mut command = Command::new(crate::host_install::git_binary()?);
                command
                    .args(["status", "--short"])
                    .current_dir(&route.collaboration_root);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    command.creation_flags(0x08000000);
                }
                let output = command.output()?;
                ensure!(
                    output.status.success(),
                    "Collaboration workspace status failed"
                );
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"mode":"collaboration_workspace","root":route.collaboration_root,"changes":String::from_utf8_lossy(&output.stdout)})
                    )?
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&crate::snapshot::diff(&snapshot)?)?
                );
            }
        }
        "apply" => {
            let snapshot = crate::snapshot::load(&fleet.directory)?;
            if let Some(route) = fleet.routes.first() {
                ensure!(
                    route.collaboration_root.canonicalize()? == snapshot.shadow.canonicalize()?,
                    "Collaboration workspace changes are already durable; apply is only for isolated source-import mode"
                );
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&crate::snapshot::apply(&snapshot)?)?
            );
        }
        "admit" => {
            ensure!(id == "task", "Use admit task <task-json>");
            let task: crate::team_policy::TaskRequest =
                read_json(request.context("A task JSON path is required")?)?;
            ensure!(
                slug(&task.task_id)
                    && task.task_id.len() <= 64
                    && (1..=86400).contains(&task.timeout_seconds),
                "Invalid task ID or timeout"
            );
            ensure!(
                task.brief_path.is_absolute() && task.brief_path.is_file(),
                "Brief path must name an existing absolute file"
            );
            let horizon = commit_horizon(&fleet, Some(task))?;
            println!("{}", serde_json::to_string_pretty(&horizon)?);
        }
        "replan" => {
            ensure!(id == "queue" && request.is_none(), "Use replan queue");
            let horizon = commit_horizon(&fleet, None)?;
            println!("{}", serde_json::to_string_pretty(&horizon)?);
        }
        "update" => {
            let task: crate::team_policy::TaskRequest =
                read_json(request.context("An updated task JSON path is required")?)?;
            ensure!(task.task_id == id, "Updated task identity differs");
            let paths = crate::agent_setup::paths()?;
            let (_, ledger, profile_identity) = crate::agent_setup::read_runtime(&paths)?;
            crate::agent_setup::transact(
                &paths,
                &profile_identity,
                Some(ledger.revision),
                |_, current| {
                    ensure!(
                        !current
                            .jobs
                            .iter()
                            .any(|job| job.parent_task_id == id && !job.attempt_ids.is_empty()),
                        "Started task scope, risk, or deadline cannot be rewritten"
                    );
                    let existing = current
                        .tasks
                        .iter_mut()
                        .find(|existing| existing.task_id == id)
                        .context("Unknown task ID")?;
                    ensure!(
                        existing.generation_id == task.generation_id
                            && existing.workspace_id == task.workspace_id
                            && existing.idempotency_key == task.idempotency_key,
                        "Task binding or idempotency identity cannot change"
                    );
                    *existing = task;
                    Ok(())
                },
            )?;
            let horizon = commit_horizon(&fleet, None)?;
            println!("{}", serde_json::to_string_pretty(&horizon)?);
        }
        "run" => {
            ensure!(
                request.is_none(),
                "Run accepts a planned visit ID without a request file"
            );
            let setup_paths = crate::agent_setup::paths()?;
            ensure!(
                setup_paths.ledger.canonicalize()? == fleet.ledger.canonicalize()?,
                "Fleet ledger path changed"
            );
            let (_, ledger, profile_identity) = crate::agent_setup::read_runtime(&setup_paths)?;
            ensure!(
                profile_identity == fleet.profile_identity
                    && fleet.policy_version == crate::team_policy::VERSION,
                "Fleet policy or capability profile is stale; finalize again"
            );
            let generation_id = fleet
                .directory
                .file_name()
                .and_then(|name| name.to_str())
                .context("Generation identity is unavailable")?;
            let pointer = ledger
                .horizons
                .iter()
                .find(|pointer| pointer.generation_id == generation_id)
                .context("No committed native horizon for this generation")?;
            ensure!(
                pointer.profile_identity == profile_identity
                    && pointer.policy_version == fleet.policy_version,
                "Committed horizon is stale"
            );
            let horizon: crate::native_scheduler::NativeHorizon =
                read_json(Path::new(&pointer.artifact_path))?;
            ensure!(
                horizon.horizon_id == pointer.id
                    && horizon.ledger_revision == pointer.committed_revision,
                "Native horizon artifact does not match the ledger commit"
            );
            let (plan, assignment) = horizon
                .plans
                .iter()
                .find_map(|plan| {
                    plan.assignments
                        .iter()
                        .find(|assignment| assignment.visit.id == id)
                        .map(|assignment| (plan, assignment))
                })
                .context("Unknown planned visit ID")?;
            ensure!(
                assignment.visit.seat != crate::types::Seat::Orchestrator,
                "Conductor visits settle through the lifecycle meter hook"
            );
            let entry = ledger
                .jobs
                .iter()
                .find(|entry| entry.id == id)
                .context("Planned visit reservation is absent")?;
            ensure!(
                entry.status == crate::agent_setup::JobStatus::Held && entry.attempt_ids.is_empty(),
                "Visit is not available for first launch"
            );
            ensure!(
                visit_ready(entry, &ledger),
                "Visit branch is inactive or dependencies lack accepted immutable output"
            );
            let route = fleet
                .routes
                .iter()
                .find(|route| {
                    route.id == assignment.route_id && route.binding_id == assignment.binding_id
                })
                .context("Assigned fixed route is unavailable")?
                .clone();
            let mut job = ledger
                .tasks
                .iter()
                .find(|task| task.task_id == plan.task_id)
                .context("Authorized task record is absent")?
                .clone();
            let directory = jobs.join(id);
            fs::create_dir(&directory)
                .context("Task ID already exists; use status instead of resubmitting")?;
            let mut brief = String::new();
            fs::File::open(&job.brief_path)?
                .take(1_048_577)
                .read_to_string(&mut brief)?;
            ensure!(brief.len() <= 1_048_576, "Task brief exceeds 1 MiB");
            let frozen_brief = directory.join("brief.md");
            let dependency_artifacts: Vec<_> = entry.dependencies.iter().flat_map(|dependency| ledger.jobs.iter().filter(|job| job.id == *dependency).flat_map(|job| &job.artifact_records)).map(|artifact| serde_json::json!({"id":artifact.id,"path":artifact.path,"digest":artifact.digest,"producer_attempt_id":artifact.producer_attempt_id})).collect();
            let scoped = format!(
                "Assigned visit: {}\nSeat: {}\nCriteria: {}\nImmutable dependency artifacts: {}\nExpected output: one bounded artifact for this seat and criteria, or a precise blocker. Keep all user work inside the pinned collaboration workspace and its documented projects, research, decisions, and deliverables layout. Use no tests or validator scaffolding. Do not commit, push, publish, deploy, change production, spawn subagents, delegate, or invoke another model/harness. Source code uses clear names without explanatory prose comments. Net Research must produce the dispatcher evidence packet plus readable findings in the collaboration research area. Report actual artifacts and blockers without inferring success from process exit.\n\nParent task brief:\n{brief}",
                assignment.visit.id,
                assignment.visit.seat.name(),
                assignment.visit.criteria.join(", "),
                serde_json::to_string(&dependency_artifacts)?
            );
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&frozen_brief)?
                .write_all(scoped.as_bytes())?;
            job.brief_path = frozen_brief;
            let policy_identity = format!(
                "{}:{}:{}",
                horizon.policy_version, profile_identity, pointer.committed_revision
            );
            write_new(
                &directory.join("request.json"),
                &QueuedJob {
                    route,
                    job,
                    ledger_job_id: id.to_owned(),
                    policy_identity,
                    profile_identity,
                    horizon_id: horizon.horizon_id.clone(),
                },
            )?;
            let mut command = Command::new(std::env::current_exe()?);
            command
                .arg("--fleet-worker")
                .arg(manifest)
                .arg(id)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x08000000);
            }
            let child = command.spawn()?;
            write_new(
                &directory.join("controller.json"),
                &ControllerIdentity::child(&child)?,
            )?;
            println!("{}", directory.display());
        }
        "settle" => {
            let settlement: crate::native_reconciliation::SettlementInput =
                read_json(request.context("A settlement JSON path is required")?)?;
            let setup_paths = crate::agent_setup::paths()?;
            let (_, ledger, profile_identity) = crate::agent_setup::read_runtime(&setup_paths)?;
            if let Some(job) = ledger.jobs.iter().find(|job| job.id == id)
                && job.status == crate::agent_setup::JobStatus::Held
                && job.seat == Some(crate::types::Seat::Orchestrator)
                && ledger
                    .tasks
                    .iter()
                    .any(|task| task.task_id == job.parent_task_id)
            {
                ensure!(
                    visit_ready(job, &ledger),
                    "Conductor visit dependencies or branch are not ready"
                );
            }
            let publication = if settlement.outcome.artifact_accepted
                && ledger
                    .jobs
                    .iter()
                    .find(|job| job.id == id)
                    .and_then(|job| job.seat)
                    == Some(crate::types::Seat::NetResearch)
            {
                let packet = settlement
                    .research_packet
                    .clone()
                    .context("Accepted Net Research requires an evidence packet")?;
                let findings_path = settlement
                    .artifacts
                    .first()
                    .map(|artifact| Path::new(&artifact.path))
                    .context("Accepted Net Research requires readable findings")?;
                let mut findings = String::new();
                fs::File::open(findings_path)?
                    .take(1_048_577)
                    .read_to_string(&mut findings)?;
                ensure!(
                    findings.len() <= 1_048_576,
                    "Research findings exceed 1 MiB"
                );
                let research_job = ledger
                    .jobs
                    .iter()
                    .find(|job| job.id == id)
                    .context("Research task identity is absent")?;
                let route = fleet
                    .routes
                    .iter()
                    .find(|route| {
                        route.id == research_job.route_id
                            && route.binding_id == research_job.binding_id
                    })
                    .context("Pinned research route is unavailable")?;
                Some((
                    crate::workspace::resolve_root(&route.collaboration_root)?,
                    research_job.parent_task_id.clone(),
                    packet,
                    findings,
                ))
            } else {
                None
            };
            crate::agent_setup::transact(
                &setup_paths,
                &profile_identity,
                Some(ledger.revision),
                |profile, current| {
                    let accepted = crate::native_reconciliation::apply_settlement(
                        profile,
                        current,
                        id,
                        &settlement,
                    )?;
                    if let (Some(reference), Some((workspace, task_id, packet, findings))) =
                        (accepted, publication.as_ref())
                    {
                        let published = crate::workspace::publish_research(
                            workspace, task_id, packet, &reference, findings,
                        )?;
                        let job = current
                            .jobs
                            .iter_mut()
                            .find(|job| job.id == id)
                            .context("Settled research visit is absent")?;
                        if let Some(artifact) = job
                            .artifact_records
                            .iter_mut()
                            .find(|artifact| artifact.id == published.id)
                        {
                            *artifact = published;
                        }
                    }
                    Ok(())
                },
            )?;
            let horizon = commit_horizon(&fleet, None)?;
            println!("{}", serde_json::to_string_pretty(&horizon)?);
        }
        "reconcile" => {
            ensure!(id == "meter", "Use reconcile meter <observation-json>");
            let observation: crate::native_reconciliation::MeterObservation =
                read_json(request.context("A meter observation JSON path is required")?)?;
            let setup_paths = crate::agent_setup::paths()?;
            let (_, ledger, profile_identity) = crate::agent_setup::read_runtime(&setup_paths)?;
            crate::agent_setup::transact(
                &setup_paths,
                &profile_identity,
                Some(ledger.revision),
                |profile, current| {
                    crate::native_reconciliation::apply_observation(profile, current, &observation)
                },
            )?;
            let horizon = commit_horizon(&fleet, None)?;
            println!("{}", serde_json::to_string_pretty(&horizon)?);
        }
        "status" => {
            let directory = jobs.join(id);
            if !directory.is_dir() {
                let paths = crate::agent_setup::paths()?;
                let (_, ledger, _) = crate::agent_setup::read_runtime(&paths)?;
                let values: Vec<_> = ledger.jobs.iter().filter(|job| job.id == id || job.parent_task_id == id).map(|job| serde_json::json!({"visit_id":job.id,"task_id":job.parent_task_id,"seat":job.seat,"status":job.status,"binding_id":job.binding_id,"holds":job.holds,"scheduled_start":job.scheduled_start,"scheduled_end":job.scheduled_end,"artifacts":job.artifact_records})).collect();
                ensure!(!values.is_empty(), "Unknown task or visit ID");
                println!("{}", serde_json::to_string_pretty(&values)?);
                return Ok(());
            }
            let result = directory.join("result.json");
            if result.is_file() {
                let value: serde_json::Value = read_json(&result)?;
                println!("{}", serde_json::to_string(&value)?);
            } else {
                let controller = directory.join("controller.json");
                let interrupted =
                    controller.is_file() && !read_json::<ControllerIdentity>(&controller)?.alive();
                println!(
                    "{}",
                    serde_json::json!({"task_id":id,"state":if interrupted{"interrupted"}else if directory.join("owner").exists(){"running"}else{"queued"}})
                );
            }
        }
        "cancel" => {
            let setup_paths = crate::agent_setup::paths()?;
            let (_, ledger, profile_identity) = crate::agent_setup::read_runtime(&setup_paths)?;
            ensure!(
                ledger.tasks.iter().any(|task| task.task_id == id),
                "Unknown task ID"
            );
            for visit in ledger.jobs.iter().filter(|job| {
                job.parent_task_id == id && job.status == crate::agent_setup::JobStatus::Running
            }) {
                let directory = jobs.join(&visit.id);
                if directory.is_dir() {
                    OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(directory.join("cancel"))?;
                }
            }
            crate::agent_setup::transact(
                &setup_paths,
                &profile_identity,
                Some(ledger.revision),
                |_, current| {
                    if let Some(task) = current.tasks.iter_mut().find(|task| task.task_id == id) {
                        task.ready = false;
                    }
                    for visit in current
                        .jobs
                        .iter_mut()
                        .filter(|job| job.parent_task_id == id && job.attempt_ids.is_empty())
                    {
                        visit.status = crate::agent_setup::JobStatus::Cancelled;
                        visit.holds.clear();
                    }
                    Ok(())
                },
            )?;
            let horizon = commit_horizon(&fleet, None)?;
            println!("{}", serde_json::to_string_pretty(&horizon)?);
        }
        _ => anyhow::bail!(
            "Supported operations: finalize, login, admit, run, settle, reconcile, status, cancel, diff, report, apply"
        ),
    }
    Ok(())
}

pub fn worker(manifest: &Path, task_id: &str) -> Result<()> {
    ensure!(slug(task_id), "Invalid task ID");
    let fleet = load_fleet(manifest)?;
    let directory = fleet.directory.join("jobs").join(task_id);
    let queued: QueuedJob = read_json(&directory.join("request.json"))?;
    ensure!(
        queued.policy_identity.starts_with(&fleet.policy_version),
        "Queued policy identity is stale"
    );
    let identity = ControllerIdentity::current()?;
    let setup_paths = crate::agent_setup::paths()?;
    crate::agent_setup::transact(
        &setup_paths,
        &queued.profile_identity,
        None,
        |profile, ledger| {
            let generation_id = fleet
                .directory
                .file_name()
                .and_then(|name| name.to_str())
                .context("Generation identity is unavailable")?;
            ensure!(
                ledger
                    .horizons
                    .iter()
                    .any(|pointer| pointer.generation_id == generation_id
                        && pointer.id == queued.horizon_id),
                "Visit assignment was replaced by a newer horizon"
            );
            let entry_index = ledger
                .jobs
                .iter()
                .position(|entry| entry.id == queued.ledger_job_id)
                .context("Reserved visit is missing")?;
            let entry = &ledger.jobs[entry_index];
            ensure!(
                entry.status == crate::agent_setup::JobStatus::Held && !entry.holds.is_empty(),
                "Visit is not atomically held for launch"
            );
            ensure!(
                entry.binding_id == queued.route.binding_id,
                "Reserved visit binding differs from the fixed route"
            );
            let visit = crate::team_policy::visits(&queued.job)
                .into_iter()
                .find(|visit| visit.id == queued.ledger_job_id)
                .context("Visit policy is absent")?;
            crate::route_qualification::qualify(
                &queued.job,
                &visit,
                &queued.route,
                profile,
                ledger,
            )?;
            ensure!(visit_ready(entry, ledger), "Visit is no longer ready");
            let now = time::OffsetDateTime::now_utc();
            if let (Some(start), Some(end)) = (&entry.scheduled_start, &entry.scheduled_end) {
                ensure!(
                    now >= time::OffsetDateTime::parse(
                        start,
                        &time::format_description::well_known::Rfc3339
                    )? && now
                        < time::OffsetDateTime::parse(
                            end,
                            &time::format_description::well_known::Rfc3339
                        )?,
                    "Visit is outside its committed active calendar interval"
                );
            }
            let entry = &mut ledger.jobs[entry_index];
            entry.status = crate::agent_setup::JobStatus::Running;
            entry.attempt_ids.push(format!(
                "attempt-{}-{}",
                queued.ledger_job_id, identity.created
            ));
            entry.worker_handle = crate::agent_setup::Fact {
                status: crate::agent_setup::FactStatus::Verified,
                value: Some(format!("{}:{}", identity.pid, identity.created)),
                evidence: vec!["dispatcher process identity".to_owned()],
            };
            entry.started_at = Some(
                time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)?,
            );
            Ok(())
        },
    )?;
    let _owner = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("owner"))?;
    let outcome = run_worker(&fleet, &directory);
    let result = match outcome {
        Ok((state, exit_code)) => JobResult { task_id:task_id.to_owned(),state,exit_code,message:"Usage settlement remains required in the shared account ledger; process success does not establish task success or quota availability.".to_owned(), output_excerpt:output_excerpt(&directory) },
        Err(error) => JobResult { task_id:task_id.to_owned(),state:"failed".to_owned(),exit_code:None,message:format!("{error:#}"),output_excerpt:output_excerpt(&directory) },
    };
    write_new(&directory.join("result.json"), &result)
}

fn output_excerpt(directory: &Path) -> String {
    let path = if directory.join("final.txt").is_file() {
        directory.join("final.txt")
    } else {
        directory.join("output.json")
    };
    let Ok(mut file) = fs::File::open(path) else {
        return String::new();
    };
    let Ok(size) = file.metadata().map(|metadata| metadata.len()) else {
        return String::new();
    };
    if file
        .seek(SeekFrom::Start(size.saturating_sub(16384)))
        .is_err()
    {
        return String::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return String::new();
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        && let Some(result) = value.get("result").and_then(serde_json::Value::as_str)
    {
        return result.to_owned();
    }
    text
}

fn run_worker(fleet: &Fleet, directory: &Path) -> Result<(String, Option<i32>)> {
    let queued: QueuedJob = read_json(&directory.join("request.json"))?;
    let route = &queued.route;
    crate::workspace::resolve_root(&route.collaboration_root)?;
    ensure!(
        route.workspace.is_dir(),
        "Pinned collaboration project workspace is unavailable"
    );
    ensure!(
        route.authenticated,
        "Confirm this generation's native login and paid account in onboarding, then finalize again"
    );
    ensure!(slug(&route.account_id), "Invalid bound account identifier");
    let _account = AccountSlot::acquire(&fleet.ledger, &route.account_id)?;
    ensure!(
        route.executable.is_absolute()
            && route.workspace.is_absolute()
            && route.home.starts_with(&fleet.directory),
        "Invalid fixed route paths"
    );
    ensure!(route.workspace.is_dir(), "Bound workspace is unavailable");
    ensure!(
        executable_identity(&route.executable)? == route.executable_identity,
        "Native executable changed; repeat onboarding and finalize"
    );
    let mut brief = String::new();
    fs::File::open(&queued.job.brief_path)?
        .take(1_048_577)
        .read_to_string(&mut brief)?;
    ensure!(brief.len() <= 1_048_576, "Task brief exceeds 1 MiB");
    prepare_home(&route.home, &fleet.directory)?;
    let mut command = clean_command(&route.executable, &route.home, &route.workspace);
    command
        .args(worker_arguments(route, &queued.job.brief_path)?)
        .stdin(
            if matches!(route.kind, HostKind::Claude | HostKind::Codex) {
                Stdio::from(fs::File::open(&queued.job.brief_path)?)
            } else {
                Stdio::null()
            },
        )
        .stdout(fs::File::create(directory.join("output.json"))?)
        .stderr(fs::File::create(directory.join("stderr.txt"))?);
    if matches!(route.kind, HostKind::Antigravity) {
        command.args([
            "--print-timeout",
            &format!("{}s", queued.job.timeout_seconds),
        ]);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000004);
    }
    let mut child = command.spawn()?;
    #[cfg(windows)]
    let _process_tree = ProcessTree::attach(&mut child)?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((
                if status.success() {
                    "completed"
                } else {
                    "failed"
                }
                .to_owned(),
                status.code(),
            ));
        }
        if directory.join("cancel").exists()
            || started.elapsed() >= Duration::from_secs(queued.job.timeout_seconds)
        {
            child.kill()?;
            child.wait()?;
            return Ok(("cancelled".to_owned(), None));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn model_arguments(kind: &HostKind, model: &str, effort: Option<&str>) -> Result<Vec<String>> {
    ensure!(
        !model.is_empty() && model.len() <= 256 && !model.starts_with('-') && !model.contains('\0'),
        "Invalid fixed model identifier"
    );
    let mut arguments = vec!["--model".to_owned(), model.to_owned()];
    if let Some(effort) = effort {
        match kind {
            HostKind::Codex => arguments.extend([
                "-c".to_owned(),
                format!("model_reasoning_effort={}", serde_json::to_string(effort)?),
            ]),
            HostKind::Claude | HostKind::Antigravity => {
                arguments.extend(["--effort".to_owned(), effort.to_owned()])
            }
            HostKind::Muse | HostKind::Grok => {
                arguments.extend(["--reasoning-effort".to_owned(), effort.to_owned()])
            }
            _ => anyhow::bail!("Unsupported fixed native model mapping"),
        }
    }
    Ok(arguments)
}

fn worker_arguments(route: &Route, brief: &Path) -> Result<Vec<String>> {
    let mut arguments = match route.kind {
        HostKind::Codex | HostKind::Muse => vec!["exec".to_owned()],
        HostKind::Claude => vec!["--safe-mode".to_owned()],
        HostKind::Grok | HostKind::Antigravity => Vec::new(),
        _ => anyhow::bail!("Unsupported isolated native worker"),
    };
    arguments.extend(model_arguments(
        &route.kind,
        &route.model,
        route.effort.as_deref(),
    )?);
    match route.kind {
        HostKind::Claude => arguments.extend([
            "--print".to_owned(),
            "--output-format".to_owned(),
            "json".to_owned(),
        ]),
        HostKind::Codex => arguments.extend([
            "-c".to_owned(),
            "default_permissions=\":workspace\"".to_owned(),
            "--ignore-user-config".to_owned(),
            "--ignore-rules".to_owned(),
            "--disable".to_owned(),
            "multi_agent".to_owned(),
            "-c".to_owned(),
            "project_doc_max_bytes=0".to_owned(),
            "--json".to_owned(),
            "--output-last-message".to_owned(),
            brief
                .with_file_name("final.txt")
                .to_string_lossy()
                .into_owned(),
            "-".to_owned(),
        ]),
        HostKind::Muse => arguments.extend([
            "--workspace".to_owned(),
            route.workspace.to_string_lossy().into_owned(),
            "--prompt-file".to_owned(),
            brief.to_string_lossy().into_owned(),
        ]),
        HostKind::Grok => arguments.extend([
            "--no-subagents".to_owned(),
            "--output-format".to_owned(),
            "json".to_owned(),
            "--prompt-file".to_owned(),
            brief.to_string_lossy().into_owned(),
        ]),
        HostKind::Antigravity => arguments.extend([
            "--output-format".to_owned(),
            "json".to_owned(),
            "--print".to_owned(),
            format!(
                "Read the scoped task brief at {} and perform only that task.",
                brief.display()
            ),
        ]),
        _ => unreachable!(),
    }
    Ok(arguments)
}
#[cfg(windows)]
struct AccountSlot(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl AccountSlot {
    fn acquire(ledger: &Path, account: &str) -> Result<Self> {
        use sha2::{Digest, Sha256};
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
        let identity = format!(
            "{}\n{account}",
            ledger.canonicalize()?.to_string_lossy().to_lowercase()
        );
        let name = format!(
            "Global\\aitierlist-account-{}",
            hex::encode(Sha256::digest(identity.as_bytes()))
        )
        .encode_utf16()
        .chain([0])
        .collect::<Vec<_>>();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        ensure!(
            !handle.is_null(),
            "Cannot create shared account worker slot"
        );
        let status = unsafe { WaitForSingleObject(handle, 0) };
        if status != WAIT_OBJECT_0 && status != WAIT_ABANDONED {
            unsafe {
                CloseHandle(handle);
            }
            anyhow::bail!(
                "This shared account already has an active worker; queue this task until it finishes"
            );
        }
        Ok(Self(handle))
    }
}

#[cfg(windows)]
impl Drop for AccountSlot {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(not(windows))]
struct AccountSlot;

#[cfg(not(windows))]
impl AccountSlot {
    fn acquire(_ledger: &Path, _account: &str) -> Result<Self> {
        anyhow::bail!("Isolated process coordination is supported on Windows")
    }
}

#[cfg(windows)]
struct ProcessTree(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl ProcessTree {
    fn attach(child: &mut std::process::Child) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::last_os_error().into());
        }
        let tree = Self(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) != 0
                && AssignProcessToJobObject(handle, child.as_raw_handle() as _) != 0
        };
        if !configured {
            let error = std::io::Error::last_os_error();
            let _ = child.kill();
            let _ = child.wait();
            return Err(error.into());
        }
        if let Err(error) = resume_primary_thread(child.id()) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        Ok(tree)
    }
}

#[cfg(windows)]
fn resume_primary_thread(process_id: u32) -> Result<()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    ensure!(
        snapshot != INVALID_HANDLE_VALUE,
        "Cannot enumerate suspended worker thread"
    );
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut found = false;
    let mut more = unsafe { Thread32First(snapshot, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == process_id {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if !thread.is_null() {
                found = unsafe { ResumeThread(thread) } != u32::MAX;
                unsafe {
                    CloseHandle(thread);
                }
            }
            break;
        }
        more = unsafe { Thread32Next(snapshot, &mut entry) } != 0;
    }
    unsafe {
        CloseHandle(snapshot);
    }
    ensure!(found, "Cannot resume contained worker thread");
    Ok(())
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
