use crate::agent_setup::HostKind;
use crate::host_install::DiscoveredHost;
use crate::types::Table;
use eframe::egui::Context;
use serde_json::Value;
use std::collections::BTreeSet;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Checking,
    CliMissing,
    Available,
    Connected,
    CredentialsReported,
    SignInCompleted,
    ReconnectRequired,
    ManagedInCli,
    Failed,
}

impl ConnectionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Checking => "Checking…",
            Self::CliMissing => "CLI missing",
            Self::Available => "CLI available",
            Self::Connected => "Connected",
            Self::CredentialsReported => "CLI reports signed in",
            Self::SignInCompleted => "Sign-in completed",
            Self::ReconnectRequired => "Sign-in required",
            Self::ManagedInCli => "Sign-in managed in CLI",
            Self::Failed => "Unavailable",
        }
    }

    pub fn connected(self) -> bool {
        self == Self::Connected
    }
}

#[derive(Clone)]
pub struct NativeConnectionEntry {
    pub id: String,
    pub provider_id: &'static str,
    pub name: String,
    pub installed: bool,
    pub supports_status: bool,
    pub status: ConnectionStatus,
    pub busy: bool,
    pub detail: String,
    host: DiscoveredHost,
}

enum Event {
    Discovery(Result<Vec<DiscoveredHost>, String>),
    Status {
        id: String,
        result: Result<ConnectionStatus, String>,
    },
    Launched {
        id: String,
        kind: LaunchKind,
        result: Result<String, String>,
    },
}

pub struct NativeConnections {
    context: Context,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    entries: Vec<NativeConnectionEntry>,
    active: BTreeSet<String>,
    discovery_started: bool,
    discovery_active: bool,
    discovery_error: Option<String>,
}

impl NativeConnections {
    pub fn new(context: Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            context,
            sender,
            receiver,
            entries: Vec::new(),
            active: BTreeSet::new(),
            discovery_started: false,
            discovery_active: false,
            discovery_error: None,
        }
    }

    pub fn ensure_discovery(&mut self) {
        if self.discovery_started {
            return;
        }
        self.discovery_started = true;
        self.start_discovery();
    }

    pub fn rediscover(&mut self) -> Result<(), String> {
        if self.discovery_active || !self.active.is_empty() {
            return Err("Wait for the active native CLI operation to finish.".to_owned());
        }
        self.start_discovery();
        Ok(())
    }

    fn start_discovery(&mut self) {
        self.discovery_active = true;
        self.discovery_error = None;
        let sender = self.sender.clone();
        let context = self.context.clone();
        std::thread::spawn(move || {
            let result = crate::host_install::discover(&Table::empty())
                .map(|discovery| {
                    discovery
                        .hosts
                        .into_iter()
                        .filter(|host| supported(host.kind))
                        .collect()
                })
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(Event::Discovery(result));
            context.request_repaint();
        });
    }

    pub fn pump(&mut self) {
        let mut refresh = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Discovery(result) => self.finish_discovery(result),
                Event::Status { id, result } => {
                    self.active.remove(&id);
                    if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
                        entry.busy = false;
                        match result {
                            Ok(status) => {
                                entry.status = status;
                                entry.detail = status_detail(entry.host.kind, status).to_owned();
                            }
                            Err(error) => {
                                entry.status = ConnectionStatus::Failed;
                                entry.detail = error;
                            }
                        }
                    }
                }
                Event::Launched { id, kind, result } => {
                    self.active.remove(&id);
                    if let Some(entry) = self.entries.iter_mut().find(|entry| entry.id == id) {
                        entry.busy = false;
                        match result {
                            Ok(detail) => {
                                entry.detail = detail;
                                if matches!(kind, LaunchKind::Connect) {
                                    entry.status = if status_arguments(entry.host.kind).is_some() {
                                        refresh.push(entry.id.clone());
                                        ConnectionStatus::Available
                                    } else if reports_login_completion(entry.host.kind) {
                                        ConnectionStatus::SignInCompleted
                                    } else {
                                        ConnectionStatus::ManagedInCli
                                    };
                                }
                            }
                            Err(error) => {
                                entry.status = ConnectionStatus::Failed;
                                entry.detail = error;
                            }
                        }
                    }
                }
            }
        }
        for id in refresh {
            let _ = self.refresh_status(&id);
        }
    }

    pub fn entries(&self) -> &[NativeConnectionEntry] {
        &self.entries
    }

    pub fn loading(&self) -> bool {
        self.discovery_active && self.entries.is_empty()
    }

    pub fn discovering(&self) -> bool {
        self.discovery_active
    }

    pub fn discovery_error(&self) -> Option<&str> {
        self.discovery_error.as_deref()
    }

    pub fn connect(&mut self, id: &str) -> Result<(), String> {
        self.launch(id, LaunchKind::Connect)
    }

    pub fn open_cli(&mut self, id: &str) -> Result<(), String> {
        self.launch(id, LaunchKind::Open)
    }

    pub fn refresh_status(&mut self, id: &str) -> Result<(), String> {
        let host = self.available_host(id)?.clone();
        let arguments = status_arguments(host.kind)
            .ok_or_else(|| "This CLI manages sign-in in its own window.".to_owned())?;
        self.mark_busy(id, Some(ConnectionStatus::Checking))?;
        let sender = self.sender.clone();
        let context = self.context.clone();
        std::thread::spawn(move || {
            let result = read_status(&host, arguments);
            let _ = sender.send(Event::Status {
                id: host.id,
                result,
            });
            context.request_repaint();
        });
        Ok(())
    }

    fn finish_discovery(&mut self, result: Result<Vec<DiscoveredHost>, String>) {
        self.discovery_active = false;
        let hosts = match result {
            Ok(hosts) => hosts,
            Err(error) => {
                self.discovery_error = Some(error);
                return;
            }
        };
        self.entries = hosts
            .into_iter()
            .map(|host| {
                let installed = host.installed && host.executable.is_some();
                let status = if !installed {
                    ConnectionStatus::CliMissing
                } else if status_arguments(host.kind).is_some() {
                    ConnectionStatus::Available
                } else {
                    ConnectionStatus::ManagedInCli
                };
                NativeConnectionEntry {
                    id: host.id.clone(),
                    provider_id: provider_id(host.kind),
                    name: host.name.clone(),
                    installed,
                    supports_status: status_arguments(host.kind).is_some(),
                    status,
                    busy: false,
                    detail: status_detail(host.kind, status).to_owned(),
                    host,
                }
            })
            .collect();
        let status_ids = self
            .entries
            .iter()
            .filter(|entry| entry.installed && status_arguments(entry.host.kind).is_some())
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        for id in status_ids {
            let _ = self.refresh_status(&id);
        }
    }

    fn launch(&mut self, id: &str, kind: LaunchKind) -> Result<(), String> {
        let host = self.available_host(id)?.clone();
        self.mark_busy(id, None)?;
        let sender = self.sender.clone();
        let context = self.context.clone();
        std::thread::spawn(move || {
            let result = spawn_interactive(&host, kind);
            let _ = sender.send(Event::Launched {
                id: host.id,
                kind,
                result,
            });
            context.request_repaint();
        });
        Ok(())
    }

    fn available_host(&self, id: &str) -> Result<&DiscoveredHost, String> {
        if self.discovery_active {
            return Err("Wait for native CLI discovery to finish.".to_owned());
        }
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or_else(|| "Unknown native CLI profile.".to_owned())?;
        if !entry.installed {
            return Err("The native CLI is not installed.".to_owned());
        }
        if entry.busy || self.active.contains(id) {
            return Err("An operation is already active for this CLI profile.".to_owned());
        }
        Ok(&entry.host)
    }

    fn mark_busy(&mut self, id: &str, status: Option<ConnectionStatus>) -> Result<(), String> {
        if !self.active.insert(id.to_owned()) {
            return Err("An operation is already active for this CLI profile.".to_owned());
        }
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| "Unknown native CLI profile.".to_owned())?;
        entry.busy = true;
        if let Some(status) = status {
            entry.status = status;
        }
        entry.detail = "Waiting for the native CLI…".to_owned();
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum LaunchKind {
    Connect,
    Open,
}

fn supported(kind: HostKind) -> bool {
    matches!(
        kind,
        HostKind::Codex
            | HostKind::Claude
            | HostKind::Muse
            | HostKind::Antigravity
            | HostKind::Grok
    )
}

fn provider_id(kind: HostKind) -> &'static str {
    match kind {
        HostKind::Codex => "openai",
        HostKind::Claude => "anthropic",
        HostKind::Muse => "muse",
        HostKind::Antigravity => "google",
        HostKind::Grok => "xai",
        _ => unreachable!(),
    }
}

fn status_arguments(kind: HostKind) -> Option<&'static [&'static str]> {
    match kind {
        HostKind::Codex => Some(&["login", "status"]),
        HostKind::Claude => Some(&["auth", "status"]),
        _ => None,
    }
}

fn login_arguments(kind: HostKind) -> &'static [&'static str] {
    match kind {
        HostKind::Codex => &["login"],
        HostKind::Claude => &["auth", "login"],
        HostKind::Muse => &["login"],
        HostKind::Antigravity => &[],
        HostKind::Grok => &["login", "--oauth"],
        _ => unreachable!(),
    }
}

fn status_detail(kind: HostKind, status: ConnectionStatus) -> &'static str {
    match status {
        ConnectionStatus::CliMissing => "Install the native CLI to connect this subscription.",
        ConnectionStatus::Available if matches!(kind, HostKind::Codex | HostKind::Claude) => {
            "The CLI is available. Check or start native subscription sign-in."
        }
        ConnectionStatus::Connected => "Native subscription sign-in is active.",
        ConnectionStatus::CredentialsReported => {
            "The profile-specific CLI reports stored sign-in material; current validity was not independently verified."
        }
        ConnectionStatus::SignInCompleted => {
            "The native CLI reported successful sign-in completion; authentication was not independently verified."
        }
        ConnectionStatus::ReconnectRequired => "Open the native sign-in flow to reconnect.",
        ConnectionStatus::ManagedInCli => "Sign-in stays with the native CLI in its own window.",
        ConnectionStatus::Failed => "The native CLI operation failed.",
        ConnectionStatus::Checking => "Waiting for the native CLI…",
        ConnectionStatus::Available => "The native CLI is available.",
    }
}

fn reports_login_completion(kind: HostKind) -> bool {
    matches!(kind, HostKind::Muse | HostKind::Grok)
}

fn command_for(host: &DiscoveredHost) -> Result<Command, String> {
    let executable = host
        .executable
        .as_ref()
        .ok_or_else(|| "The native CLI executable is unavailable.".to_owned())?;
    let mut command = Command::new(executable);
    command.envs(&host.environment);
    for name in excluded_auth_environment(host.kind) {
        command.env_remove(name);
    }
    let home = directories::BaseDirs::new()
        .map(|directories| directories.home_dir().to_path_buf())
        .ok_or_else(|| "The user profile directory is unavailable.".to_owned())?;
    command.current_dir(home);
    Ok(command)
}

fn excluded_auth_environment(kind: HostKind) -> &'static [&'static str] {
    match kind {
        HostKind::Codex => &["OPENAI_API_KEY", "CODEX_API_KEY"],
        HostKind::Claude => &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
        ],
        HostKind::Muse => &["META_API_KEY"],
        HostKind::Antigravity => &["GEMINI_API_KEY", "AGY_ADC_AUTH"],
        HostKind::Grok => &["XAI_API_KEY"],
        _ => &[],
    }
}

fn spawn_interactive(host: &DiscoveredHost, kind: LaunchKind) -> Result<String, String> {
    let mut command = command_for(host)?;
    match kind {
        LaunchKind::Connect => {
            command.args(login_arguments(host.kind));
        }
        LaunchKind::Open => {
            command.args(&host.arguments);
        }
    }
    command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000010);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not open {}: {error}", host.name))?;
    if matches!(kind, LaunchKind::Connect) {
        let status = child
            .wait()
            .map_err(|error| format!("Could not wait for {} sign-in: {error}", host.name))?;
        if !status.success() {
            return Err(format!("{} sign-in exited without completing.", host.name));
        }
    }
    Ok(match kind {
        LaunchKind::Connect if reports_login_completion(host.kind) => {
            "The native CLI reported successful sign-in completion; authentication was not independently verified.".to_owned()
        }
        LaunchKind::Connect => "Complete sign-in in the native CLI.".to_owned(),
        LaunchKind::Open => "Opened the native CLI.".to_owned(),
    })
}

fn read_status(
    host: &DiscoveredHost,
    arguments: &'static [&'static str],
) -> Result<ConnectionStatus, String> {
    let mut command = command_for(host)?;
    command
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not check {} sign-in: {error}", host.name))?;
    let deadline = Instant::now() + Duration::from_secs(8);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("The native sign-in check timed out.".to_owned());
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Could not check native sign-in: {error}"));
            }
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        pipe.read_to_string(&mut stdout)
            .map_err(|_| "Could not read native sign-in status.".to_owned())?;
    }
    if let Some(mut pipe) = child.stderr.take() {
        pipe.read_to_string(&mut stderr)
            .map_err(|_| "Could not read native sign-in status.".to_owned())?;
    }
    Ok(match host.kind {
        HostKind::Codex => codex_status(status.success(), &stdout, &stderr),
        HostKind::Claude => claude_status(status.success(), &stdout),
        _ => ConnectionStatus::ManagedInCli,
    })
}

fn codex_status(success: bool, stdout: &str, stderr: &str) -> ConnectionStatus {
    let output = format!("{stdout}\n{stderr}").to_ascii_lowercase();
    if output.contains("not logged in")
        || output.contains("login required")
        || output.contains("api key")
    {
        ConnectionStatus::ReconnectRequired
    } else if success
        && (output.contains("logged in using chatgpt") || output.contains("logged in with chatgpt"))
    {
        ConnectionStatus::CredentialsReported
    } else if !success {
        ConnectionStatus::Failed
    } else {
        ConnectionStatus::Available
    }
}

fn claude_status(success: bool, stdout: &str) -> ConnectionStatus {
    let Ok(value) = serde_json::from_str::<Value>(stdout) else {
        return if success {
            ConnectionStatus::Available
        } else {
            ConnectionStatus::Failed
        };
    };
    let logged_in = value
        .get("loggedIn")
        .or_else(|| value.get("logged_in"))
        .and_then(Value::as_bool);
    let subscription = value
        .get("subscriptionType")
        .or_else(|| value.get("subscription_type"))
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty());
    let auth_method = value
        .get("authMethod")
        .or_else(|| value.get("auth_method"))
        .and_then(Value::as_str)
        .map(|value| value.to_ascii_lowercase());
    let subscription_auth = auth_method.as_deref().map_or(subscription, |method| {
        matches!(method, "claude.ai" | "claudeai")
    });
    if !success && logged_in != Some(false) {
        return ConnectionStatus::Failed;
    }
    match (logged_in, subscription_auth, auth_method.is_some()) {
        (Some(true), true, _) => ConnectionStatus::Connected,
        (Some(false), _, _) => ConnectionStatus::ReconnectRequired,
        (Some(true), false, true) => ConnectionStatus::ReconnectRequired,
        _ if success => ConnectionStatus::Available,
        _ => ConnectionStatus::Failed,
    }
}
