use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agent_setup::{self, HostKind, SetupPaths, SetupState};
use crate::types::{Seat, Table};

#[derive(Clone, Serialize, Deserialize)]
pub struct DiscoveredHost {
    pub id: String,
    pub kind: HostKind,
    pub name: String,
    pub executable: Option<PathBuf>,
    pub environment: BTreeMap<String, String>,
    pub arguments: Vec<String>,
    pub installed: bool,
    pub detection: String,
    pub detected_path: Option<PathBuf>,
    pub auth_status: String,
    pub native_adapter: bool,
    pub message: String,
    pub isolation_supported: bool,
    pub isolation_message: String,
    pub isolation_version: Option<String>,
    pub isolation_identity: Option<String>,
}

pub struct InstallOptions {
    pub isolated: bool,
}

#[derive(Clone)]
pub struct Discovery {
    pub hosts: Vec<DiscoveredHost>,
    pub setup: SetupState,
    pub paths: SetupPaths,
    pub recommended_host: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverySeed {
    pub paths: SetupPaths,
    pub hosts: Vec<DiscoveredHost>,
}

pub fn refresh_discovery(table: &Table, seed: DiscoverySeed) -> Result<Discovery> {
    let setup = agent_setup::load_setup_at(table, &seed.paths)?;
    finish_discovery(table, seed.paths, setup, seed.hosts)
}

#[derive(Clone)]
struct StagedFile {
    path: PathBuf,
    root: PathBuf,
    bytes: Vec<u8>,
    executable: bool,
}

#[derive(Clone)]
struct StagedHost {
    files: Vec<StagedFile>,
    launcher: InstalledLauncher,
}

#[derive(Clone)]
pub struct InstallPreview {
    pub isolated: bool,
    pub directory: PathBuf,
    pub policy_path: PathBuf,
    pub destinations: Vec<PathBuf>,
    pub impacts: Vec<String>,
    pub commands: Vec<String>,
    pub host_ids: Vec<String>,
    pub add_to_user_path: bool,
    common_files: Vec<StagedFile>,
    hosts: Vec<StagedHost>,
    bin: PathBuf,
}

#[derive(Clone)]
pub struct InstalledLauncher {
    pub host_id: String,
    pub adapter: String,
    pub path: PathBuf,
    pub command: String,
}

#[derive(Clone)]
pub struct InstallFailure {
    pub host_id: String,
    pub message: String,
}

#[derive(Clone)]
pub struct InstallResult {
    pub isolated: bool,
    pub directory: PathBuf,
    pub policy_path: PathBuf,
    pub launchers: Vec<InstalledLauncher>,
    pub failures: Vec<InstallFailure>,
    pub path_added: Option<PathBuf>,
    pub notes: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LaunchManifest {
    version: u32,
    executable: PathBuf,
    arguments: Vec<String>,
    environment: BTreeMap<String, String>,
    policy_path: PathBuf,
    #[serde(default)]
    isolated_home: Option<PathBuf>,
    #[serde(default)]
    workspace: Option<PathBuf>,
    #[serde(default)]
    kind: Option<HostKind>,
    #[serde(default)]
    isolation_identity: Option<String>,
    #[serde(default)]
    host_id: Option<String>,
}

fn cloud_path(path: &Path) -> bool {
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    text.split('\\').any(|part| {
        matches!(
            part,
            "iclouddrive"
                | "icloud photos archive"
                | "onedrive"
                | "dropbox"
                | "proton drive"
                | "google drive"
        ) || part.starts_with("onedrive - ")
    }) || ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"]
        .iter()
        .any(|name| {
            std::env::var_os(name).is_some_and(|root| {
                let root = root.to_string_lossy().replace('/', "\\").to_lowercase();
                !root.is_empty() && (text == root || text.starts_with(&format!("{root}\\")))
            })
        })
}

fn executable(path: PathBuf) -> Option<PathBuf> {
    if cloud_path(&path) || !path.is_file() {
        return None;
    }
    let resolved = path.canonicalize().ok()?;
    (!cloud_path(&resolved)).then_some(path)
}

fn known_desktop(path: &Path) -> bool {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return false;
    };
    let desktop = local.join("Programs/OpenCode/OpenCode.exe");
    if !path.is_absolute() || cloud_path(path) || cloud_path(&desktop) {
        return false;
    }
    let (Ok(candidate), Ok(desktop)) = (path.canonicalize(), desktop.canonicalize()) else {
        return false;
    };
    if cfg!(windows) {
        candidate
            .to_string_lossy()
            .eq_ignore_ascii_case(&desktop.to_string_lossy())
    } else {
        candidate == desktop
    }
}

fn on_path(names: &[&str]) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .filter(|directory| directory.is_absolute() && !cloud_path(directory))
        .find_map(|directory| {
            names.iter().find_map(|name| {
                executable(directory.join(name)).filter(|path| !known_desktop(path))
            })
        })
}

fn standalone(home: &Path) -> Option<PathBuf> {
    let releases = home.join("packages/standalone/releases");
    if cloud_path(&releases) {
        return None;
    }
    let mut candidates = fs::read_dir(releases)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let version = name
                .split('-')
                .next()?
                .split('.')
                .map(str::parse::<u32>)
                .collect::<std::result::Result<Vec<_>, _>>()
                .ok()?;
            let binary = executable(entry.path().join(if cfg!(windows) {
                "bin/codex.exe"
            } else {
                "bin/codex"
            }))?;
            Some((version, binary))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| right.0.cmp(&left.0));
    candidates.into_iter().next().map(|(_, path)| path)
}

fn host(
    id: &str,
    kind: HostKind,
    binary: Option<PathBuf>,
    environment: BTreeMap<String, String>,
    native: bool,
    message: &str,
) -> DiscoveredHost {
    DiscoveredHost {
        isolation_supported: false,
        isolation_message: "Isolation capability has not been established".to_owned(),
        isolation_version: None,
        isolation_identity: None,
        id: id.to_string(),
        name: kind.name().to_string(),
        kind,
        installed: binary.is_some(),
        detection: if binary.is_some() {
            "executable"
        } else {
            "absent"
        }
        .to_owned(),
        detected_path: binary.clone(),
        executable: binary,
        environment,
        arguments: Vec::new(),
        auth_status: "Not inspected; confirm the account during onboarding".to_string(),
        native_adapter: native,
        message: message.to_string(),
    }
}

fn native_command(path: &Path) -> bool {
    !cfg!(windows)
        || path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
}

fn multi_provider_host(home: &Path, local: &Path, kind: HostKind, id: &str) -> DiscoveredHost {
    let names = if cfg!(windows) {
        vec![
            format!("{id}.exe"),
            format!("{id}.cmd"),
            format!("{id}.ps1"),
        ]
    } else {
        vec![id.to_owned()]
    };
    let path_names: Vec<_> = names.iter().map(String::as_str).collect();
    let detected = (!matches!(kind, HostKind::Buzz))
        .then(|| on_path(&path_names))
        .flatten();
    let package = match kind {
        HostKind::OpenCode => Some((
            "OPENCODE_BIN_PATH",
            "opencode-windows-x64/bin/opencode.exe",
            "opencode-ai",
        )),
        HostKind::Cline => Some((
            "CLINE_BIN_PATH",
            "@cline/cli-windows-x64/bin/cline.exe",
            "cline",
        )),
        _ => None,
    };
    let configured_override = package.and_then(|(variable, _, _)| {
        std::env::var_os(variable).map(|value| (variable, PathBuf::from(value)))
    });
    if let Some((variable, path)) = &configured_override
        && (!path.is_absolute()
            || !native_command(path)
            || executable(path.clone()).is_none()
            || known_desktop(path))
    {
        let mut found = host(id, kind, None, BTreeMap::new(), false, "");
        found.detection = if known_desktop(path) {
            "desktop_only"
        } else {
            "invalid_override"
        }
        .to_owned();
        found.detected_path = Some(path.clone());
        found.message = format!(
            "Configured {variable}={} needs setup: an available absolute CLI executable is required; the known OpenCode desktop app is not a CLI. No PATH or package fallback was substituted.",
            path.display()
        );
        return found;
    }
    let overridden = configured_override.map(|(_, path)| path);
    let sidecar = || {
        let (_, relative, wrapper) = package?;
        if !cfg!(all(windows, target_arch = "x86_64")) {
            return None;
        }
        let mut roots = vec![
            std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData/Roaming"))
                .join("npm"),
        ];
        if let Some(parent) = detected.as_ref().and_then(|path| path.parent()) {
            roots.insert(0, parent.to_path_buf());
        }
        roots
            .into_iter()
            .filter(|root| root.is_absolute())
            .find_map(|root| {
                executable(root.join("node_modules").join(relative))
                    .or_else(|| {
                        executable(
                            root.join("node_modules")
                                .join(wrapper)
                                .join("node_modules")
                                .join(relative),
                        )
                    })
                    .or_else(|| {
                        if !matches!(kind, HostKind::OpenCode) {
                            return None;
                        }
                        let baseline = "opencode-windows-x64-baseline/bin/opencode.exe";
                        executable(root.join("node_modules").join(baseline)).or_else(|| {
                            executable(
                                root.join("node_modules/opencode-ai/node_modules")
                                    .join(baseline),
                            )
                        })
                    })
            })
    };
    let known_binary = || {
        if matches!(kind, HostKind::Buzz) {
            return None;
        }
        [
            home.join(".local/bin"),
            home.join(".bun/bin"),
            home.join(format!(".{id}/bin")),
            home.join(format!(".local/share/{id}/bin")),
            home.join(format!("scoop/apps/{id}/current")),
        ]
        .into_iter()
        .find_map(|directory| {
            names
                .iter()
                .find_map(|name| executable(directory.join(name)))
        })
    };
    let candidate = overridden
        .or_else(|| detected.clone().filter(|path| native_command(path)))
        .or_else(sidecar)
        .or_else(known_binary)
        .or(detected)
        .filter(|path| !known_desktop(path));
    let shim = candidate.as_ref().is_some_and(|path| !native_command(path));
    let mut found = host(
        id,
        kind,
        candidate.clone().filter(|_| !shim),
        BTreeMap::new(),
        false,
        "Multi-provider harness: the configured model, account, connector and billing are unverified. Subscription access does not follow from the harness name.",
    );
    found.installed = candidate.is_some();
    found.detected_path = candidate.clone();
    if let Some(path) = candidate {
        found.detection = if shim { "shim" } else { "executable" }.to_owned();
        if shim {
            found.message = format!(
                "CLI shim detected at {}. Resolve a supported native executable or verified runtime entry point during setup; shell shims are not evaluated. Provider, account and billing remain unverified.",
                path.display()
            );
        }
    } else {
        let mut configurations = vec![
            home.join(format!(".config/{id}")),
            home.join(format!(".{id}")),
        ];
        if matches!(kind, HostKind::OpenCode) {
            configurations.push(local.join("ai.opencode.desktop"));
        }
        if matches!(kind, HostKind::Aider) {
            configurations.push(home.join(".aider.conf.yml"));
        }
        let config = configurations
            .into_iter()
            .find(|path| !cloud_path(path) && path.exists());
        let desktop = match kind {
            HostKind::Buzz => executable(local.join("Buzz/buzz-desktop.exe")),
            HostKind::OpenCode => executable(local.join("Programs/OpenCode/OpenCode.exe")),
            _ => None,
        };
        if let Some(path) = desktop.clone().or(config) {
            found.detection = if desktop.as_ref() == Some(&path) {
                "desktop_only"
            } else {
                "config_only"
            }
            .to_owned();
            found.detected_path = Some(path.clone());
            found.message = format!(
                "{} footprint at {}; no supported CLI executable was found. Desktop, IDE or configuration files do not establish CLI launch support or paid access.",
                kind.name(),
                path.display()
            );
        }
    }
    found
}
pub fn discover(table: &Table) -> Result<Discovery> {
    let home = directories::BaseDirs::new()
        .context("Cannot locate the user directory")?
        .home_dir()
        .to_path_buf();
    let local = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let paths = agent_setup::paths()?;
    let setup = agent_setup::load_setup(table)?;
    let mut hosts = Vec::new();
    let codex_path = on_path(if cfg!(windows) {
        &["codex.exe"]
    } else {
        &["codex"]
    });
    let mut codex_homes = Vec::new();
    if let Some(value) = std::env::var_os("CODEX_HOME").filter(|value| !value.is_empty()) {
        codex_homes.push(("codex-environment", PathBuf::from(value)));
    }
    for (id, directory) in [
        ("codex-1", ".codex1"),
        ("codex-2", ".codex2"),
        ("codex", ".codex"),
    ] {
        let directory = home.join(directory);
        if !cloud_path(&directory)
            && directory.is_dir()
            && !codex_homes.iter().any(|(_, other)| other == &directory)
        {
            codex_homes.push((id, directory));
        }
    }
    for (id, directory) in codex_homes {
        if cloud_path(&directory) {
            continue;
        }
        if let Some(binary) = standalone(&directory).or_else(|| codex_path.clone()) {
            let mut environment = BTreeMap::new();
            environment.insert(
                "CODEX_HOME".to_string(),
                directory.to_string_lossy().into_owned(),
            );
            let mut found = host(
                id,
                HostKind::Codex,
                Some(binary),
                environment,
                false,
                "Interactive launcher reads the canonical Markdown; existing account and base instructions remain selected.",
            );
            found.name = format!("Codex ({})", directory.display());
            hosts.push(found);
        }
    }
    if !hosts
        .iter()
        .any(|entry| matches!(entry.kind, HostKind::Codex))
    {
        hosts.push(host(
            "codex",
            HostKind::Codex,
            codex_path,
            BTreeMap::new(),
            false,
            "Interactive read-file launcher; no primary --agent selector is assumed.",
        ));
    }
    let claude = executable(home.join(".local/bin/claude.exe")).or_else(|| {
        on_path(if cfg!(windows) {
            &["claude.exe"]
        } else {
            &["claude"]
        })
    });
    let mut claude_environment = BTreeMap::new();
    if let Some(value) = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|value| !value.is_empty()) {
        claude_environment.insert(
            "CLAUDE_CONFIG_DIR".to_string(),
            value.to_string_lossy().into_owned(),
        );
    }
    let claude_native = claude_environment.is_empty();
    hosts.push(host("claude", HostKind::Claude, claude, claude_environment, claude_native, "Native named primary agent when the standard config home is used; otherwise interactive read-file launcher."));
    let muse_directory = local.join("Programs/Muse");
    let muse = executable(muse_directory.join("muse.exe"))
        .or_else(|| {
            if cloud_path(&muse_directory) {
                return None;
            }
            let mut candidates = fs::read_dir(&muse_directory)
                .ok()?
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name().to_string_lossy().to_lowercase();
                    (name.starts_with("muse") && name.ends_with(".exe"))
                        .then(|| executable(entry.path()))
                        .flatten()
                })
                .collect::<Vec<_>>();
            candidates.sort();
            (candidates.len() == 1).then(|| candidates.remove(0))
        })
        .or_else(|| {
            on_path(if cfg!(windows) {
                &["muse.exe"]
            } else {
                &["muse"]
            })
        });
    hosts.push(host(
        "muse",
        HostKind::Muse,
        muse,
        BTreeMap::new(),
        false,
        "Interactive read-file launcher; does not invoke shell aliases or add --yolo.",
    ));
    let agy = executable(local.join("agy/bin/agy.exe")).or_else(|| {
        on_path(if cfg!(windows) {
            &["agy.exe"]
        } else {
            &["agy"]
        })
    });
    let mut google_environment = BTreeMap::new();
    if let Some(value) = std::env::var_os("GEMINI_CLI_HOME").filter(|value| !value.is_empty()) {
        google_environment.insert(
            "GEMINI_CLI_HOME".to_string(),
            value.to_string_lossy().into_owned(),
        );
    }
    hosts.push(host(
        "antigravity",
        HostKind::Antigravity,
        agy,
        google_environment.clone(),
        google_environment.is_empty(),
        "Antigravity native primary agent; the gemini alias may point to this executable.",
    ));
    let gemini = if cfg!(windows) {
        on_path(&["gemini.exe"]).or_else(|| on_path(&["gemini.cmd"]))
    } else {
        on_path(&["gemini"])
    };
    let gemini_shim = gemini.as_ref().is_some_and(|path| {
        cfg!(windows) && path.extension().is_some_and(|extension| extension == "cmd")
    });
    let mut gemini_host = host(
        "gemini",
        HostKind::Gemini,
        gemini.filter(|_| !gemini_shim),
        google_environment,
        false,
        "Separate Gemini CLI requires onboarding verification; no Antigravity agent format is assumed.",
    );
    if gemini_shim {
        gemini_host.installed = true;
        gemini_host.message = "Gemini CMD shim detected. Confirm a native executable and structured arguments during onboarding; automatic CMD evaluation is not supported.".to_string();
    }
    hosts.push(gemini_host);
    let mut grok_environment = BTreeMap::new();
    if let Some(value) = std::env::var_os("GROK_HOME").filter(|value| !value.is_empty()) {
        grok_environment.insert(
            "GROK_HOME".to_string(),
            value.to_string_lossy().into_owned(),
        );
    }
    let grok = executable(home.join(".grok/bin/grok.exe")).or_else(|| {
        on_path(if cfg!(windows) {
            &["grok.exe"]
        } else {
            &["grok"]
        })
    });
    hosts.push(host(
        "grok",
        HostKind::Grok,
        grok,
        grok_environment,
        false,
        "Interactive read-file launcher; native definition schema is not assumed.",
    ));
    for (kind, id) in [
        (HostKind::OpenCode, "opencode"),
        (HostKind::Aider, "aider"),
        (HostKind::Goose, "goose"),
        (HostKind::Cline, "cline"),
        (HostKind::OpenHands, "openhands"),
        (HostKind::Buzz, "buzz"),
        (HostKind::Pi, "pi"),
        (HostKind::Droid, "droid"),
    ] {
        hosts.push(multi_provider_host(&home, &local, kind, id));
    }
    finish_discovery(table, paths, setup, hosts)
}

fn finish_discovery(
    table: &Table,
    paths: SetupPaths,
    mut setup: SetupState,
    mut hosts: Vec<DiscoveredHost>,
) -> Result<Discovery> {
    if let Some(profile) = &setup.profile {
        for recorded in &profile.hosts {
            if recorded.installed.status == agent_setup::FactStatus::Unknown
                || recorded.installed.value != Some(true)
                || recorded.executable.status == agent_setup::FactStatus::Unknown
            {
                continue;
            }
            let mut binary = recorded
                .executable
                .value
                .as_ref()
                .and_then(|value| executable(PathBuf::from(value)));
            let desktop_profile = binary.as_ref().is_some_and(|path| known_desktop(path));
            let desktop_override = match recorded.kind {
                HostKind::OpenCode => std::env::var_os("OPENCODE_BIN_PATH"),
                HostKind::Cline => std::env::var_os("CLINE_BIN_PATH"),
                _ => None,
            }
            .is_some_and(|value| known_desktop(Path::new(&value)));
            let issue = if hosts
                .iter()
                .any(|host| host.kind == recorded.kind && host.detection == "invalid_override")
                || desktop_override
            {
                Some(
                    "The configured binary override needs setup; cached profile routing cannot replace it silently.",
                )
            } else if desktop_profile {
                Some(
                    "The recorded executable is the OpenCode desktop application, not a supported CLI.",
                )
            } else if !recorded.environment.iter().all(|(name, value)| {
                allowed_environment(name)
                    && Path::new(value).is_absolute()
                    && !cloud_path(Path::new(value))
            }) {
                Some("The approved host has an unsupported or nonlocal config-home override.")
            } else if !safe_arguments(&recorded.argv) {
                Some("The approved host arguments contain unsupported permission-bypass settings.")
            } else if binary.is_none() {
                Some(
                    "The approved host executable is missing or outside supported local locations.",
                )
            } else if matches!(recorded.kind, HostKind::Buzz) {
                Some("Buzz is desktop-only; no interactive orchestrator CLI adapter is verified.")
            } else if cfg!(windows)
                && binary.as_ref().is_some_and(|path| {
                    path.extension()
                        .is_none_or(|extension| !extension.eq_ignore_ascii_case("exe"))
                })
            {
                Some(
                    "A native executable or verified runtime entry point is required; shell shims are not evaluated.",
                )
            } else {
                None
            };
            if let Some(issue) = issue {
                setup.ready = false;
                setup.issues.push(format!(
                    "{}: {issue} Update onboarding before using this host.",
                    recorded.id
                ));
                setup.message = "An approved host needs onboarding repair; no replacement binding was substituted.".to_string();
                binary = None;
            }
            let native = matches!(recorded.kind, HostKind::Claude | HostKind::Antigravity)
                && recorded.environment.is_empty()
                && !recorded
                    .argv
                    .iter()
                    .any(|argument| argument == "--agent" || argument.starts_with("--agent="));
            let mut found = host(
                &recorded.id,
                recorded.kind,
                binary,
                recorded.environment.clone(),
                native,
                issue.unwrap_or("Executable, arguments, and config-home overrides come from the accepted onboarding profile."),
            );
            found.arguments = recorded.argv.clone();
            if found.executable.is_none() {
                let detected = hosts
                    .iter()
                    .find(|host| host.id == recorded.id || host.kind == recorded.kind);
                found.detection = detected
                    .filter(|host| host.detection != "absent")
                    .map(|host| host.detection.clone())
                    .unwrap_or_else(|| "profile_needs_setup".to_owned());
                found.detected_path = detected
                    .and_then(|host| host.detected_path.clone())
                    .or_else(|| recorded.executable.value.as_ref().map(PathBuf::from));
                found.installed = detected.is_some_and(|host| host.installed);
                if desktop_profile {
                    found.detection = "desktop_only".to_owned();
                    found.detected_path = recorded.executable.value.as_ref().map(PathBuf::from);
                    found.installed = false;
                }
            }
            if recorded.kind.is_multi_provider() {
                found.auth_status = recorded.active_binding_id.as_ref()
                    .and_then(|fact| fact.value.as_ref())
                    .and_then(|id| profile.bindings.iter().find(|binding| &binding.id == id))
                    .map(|binding| format!("Approved profile selects {} / account {}; subscription billing {}. Launcher preserves the configured selection.", binding.provider_id, binding.account_id, if binding.subscription_funded(true) { "confirmed" } else { "unverified or API-funded" }))
                    .unwrap_or_else(|| "Provider, account and billing unverified; onboarding required".to_owned());
            }
            if let Some(existing) = hosts.iter_mut().find(|host| host.id == recorded.id) {
                *existing = found;
            } else {
                hosts.push(found);
            }
        }
    }
    let recommended_route = table
        .portfolio
        .as_ref()
        .and_then(|portfolio| portfolio.conductor.as_ref());
    let recommended_provider = recommended_route.map(|route| route.provider_id.as_str());
    let recommended_binding = recommended_route.map(|rule| rule.binding_id.clone());
    let recommended_row = recommended_route
        .map(|rule| rule.row_index)
        .map(|index| &table.rows[index]);
    let matches_recommendation = |host: &DiscoveredHost| {
        let expected_native = host.kind.native_provider();
        if expected_native.is_some() && expected_native != recommended_provider {
            return false;
        }
        let profile = setup.profile.as_ref();
        let recorded = profile
            .and_then(|profile| profile.hosts.iter().find(|recorded| recorded.id == host.id));
        let active_id = recorded
            .and_then(|recorded| recorded.active_binding_id.as_ref())
            .and_then(|fact| fact.value.as_ref());
        let active = profile.and_then(|profile| {
            active_id.and_then(|id| profile.bindings.iter().find(|binding| &binding.id == id))
        });
        if !host.kind.is_multi_provider() {
            let routed = profile.and_then(|profile| {
                profile.bindings.iter().find(|binding| {
                    Some(&binding.id) == recommended_binding.as_ref() && binding.host_id == host.id
                })
            });
            return routed.is_some_and(|binding| binding.subscription_funded(false))
                && active.is_none_or(|binding| {
                    binding.id == recommended_binding.clone().unwrap_or_default()
                });
        }
        let Some((profile, binding)) = profile.zip(active) else {
            return false;
        };
        binding.host_id == host.id
            && Some(&binding.id) == recommended_binding.as_ref()
            && recommended_row.is_some_and(|row| {
                binding.display_model == row.model
                    && binding.display_effort.value.as_deref()
                        == Some(row.effort.as_deref().unwrap_or("none"))
            })
            && Some(binding.provider_id.as_str()) == recommended_provider
            && binding.entitlement.value == Some(true)
            && binding.native_model.value.is_some()
            && binding.model_args.value.is_some()
            && binding.effort_args.value.is_some()
            && binding.subscription_funded(true)
            && profile
                .providers
                .get(&binding.provider_id)
                .is_some_and(|provider| {
                    provider.account_id == binding.account_id
                        && provider.host_ids.contains(&host.id)
                })
            && profile.accounts.iter().any(|account| {
                account.id == binding.account_id
                    && account.authenticated.value == Some(true)
                    && table.portfolio.as_ref().is_some_and(|portfolio| {
                        portfolio.pools.iter().any(|pool| {
                            pool.provider_id == binding.provider_id
                                && account.plan.value.as_deref() == Some(pool.plan_id.as_str())
                        })
                    })
            })
    };
    let preferred_override = setup
        .profile
        .as_ref()
        .and_then(|profile| profile.orchestrator_host_override.value.as_ref())
        .filter(|value| Some(value.recommended_provider.as_str()) == recommended_provider)
        .map(|value| value.host_id.as_str());
    let recommended_host = hosts
        .iter()
        .filter(|entry| {
            entry.installed && entry.executable.is_some() && matches_recommendation(entry)
        })
        .min_by_key(|entry| {
            (
                entry.kind.is_multi_provider(),
                preferred_override != Some(entry.id.as_str()),
            )
        })
        .map(|entry| entry.id.clone());
    for batch in hosts.chunks_mut(8) {
        std::thread::scope(|scope| {
            for host in batch {
                scope.spawn(move || {
                    (host.isolation_supported, host.isolation_message) =
                        crate::isolation::capability(&host.kind, host.executable.as_deref());
                    if host.isolation_supported
                        && let Some(binary) = host.executable.as_deref()
                    {
                        host.isolation_version = crate::isolation::executable_version(binary).ok();
                        host.isolation_identity =
                            crate::isolation::executable_identity(binary).ok();
                        host.isolation_supported =
                            host.isolation_version.is_some() && host.isolation_identity.is_some();
                    }
                });
            }
        });
    }
    Ok(Discovery { hosts, setup, paths, recommended_host, notes: vec!["Discovery checks executable paths only. Authentication, delegation, quota units, and account identity remain onboarding facts.".to_string(), "Launchers preserve the caller's working directory. No profile scripts, project agent files, or base host settings are changed.".to_string()] })
}

fn unused_path(directory: &Path, stem: &str, extension: &str) -> Result<PathBuf> {
    for index in 0..10000 {
        let suffix = if index == 0 {
            String::new()
        } else {
            format!("-{index}")
        };
        let path = directory.join(format!("{stem}{suffix}{extension}"));
        if !path.try_exists()? {
            return Ok(path);
        }
    }
    bail!("Too many existing launchers for {stem}")
}

fn read_directive(policy: &Path) -> String {
    format!(
        "Before onboarding, delegation, or paid task work, read the complete canonical orchestrator instructions at {}. Follow that file as the operating policy, ask its required host/account onboarding questions, and preserve existing host safety and project instructions. Do not begin task work if the file cannot be read. This launcher supplies instructions, not verified authentication or quota.",
        policy.display()
    )
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn allowed_environment(name: &str) -> bool {
    matches!(
        name,
        "CODEX_HOME" | "CLAUDE_CONFIG_DIR" | "GROK_HOME" | "GEMINI_CLI_HOME"
    )
}

fn safe_host_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub(crate) fn git_binary() -> Result<PathBuf> {
    on_path(if cfg!(windows) {
        &["git.exe"]
    } else {
        &["git"]
    })
    .context("Git is required for the isolated project copy")
}

fn resolved_future(path: &Path) -> Result<PathBuf> {
    let existing = path
        .ancestors()
        .find(|ancestor| ancestor.exists())
        .context("Destination has no existing ancestor")?;
    Ok(existing.canonicalize()?.join(path.strip_prefix(existing)?))
}

pub(crate) fn destination_allowed(path: &Path, root: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && root.is_absolute() && path.starts_with(root),
        "Installation destination escapes its declared directory"
    );
    let resolved = resolved_future(path)?;
    ensure!(
        !cloud_path(&resolved) && resolved.starts_with(resolved_future(root)?),
        "Installation destination resolves outside its declared local directory"
    );
    Ok(())
}

fn safe_arguments(arguments: &[String]) -> bool {
    arguments.iter().all(|argument| {
        !matches!(
            argument.split('=').next().unwrap_or_default(),
            "--yolo"
                | "--dangerously-skip-permissions"
                | "--dangerously-bypass-approvals-and-sandbox"
                | "--always-approve"
                | "--disable-approval"
                | "--disable-sandbox"
        ) && !matches!(
            argument.as_str(),
            "bypassPermissions" | "danger-full-access"
        ) && !argument.contains("=bypassPermissions")
            && !argument.contains("=danger-full-access")
            && !argument.contains('\0')
    })
}

fn encoded_powershell(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(alphabet[((value >> 18) & 63) as usize] as char);
        encoded.push(alphabet[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            alphabet[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            alphabet[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

pub fn prepare_install(
    markdown: &str,
    discovery: &Discovery,
    host_ids: &[String],
    add_to_user_path: bool,
    table: &Table,
    options: InstallOptions,
) -> Result<InstallPreview> {
    prepare(
        markdown,
        discovery,
        host_ids,
        add_to_user_path,
        None,
        Some(table),
        options.isolated,
    )
}

pub(crate) fn isolated_fleet(
    discovery: &Discovery,
    table: &Table,
    directory: &Path,
) -> Result<crate::isolation::Fleet> {
    let mut fleet = crate::isolation::Fleet {
        version: 1,
        policy_version: crate::team_policy::VERSION.to_owned(),
        profile_identity: discovery
            .setup
            .profile
            .as_ref()
            .map(crate::agent_setup::profile_identity)
            .transpose()?
            .unwrap_or_default(),
        directory: directory.to_path_buf(),
        ledger: discovery.paths.ledger.clone(),
        routes: Vec::new(),
        conductors: BTreeMap::new(),
    };
    let Some(profile) = discovery
        .setup
        .profile
        .as_ref()
        .filter(|_| discovery.setup.ready)
    else {
        return Ok(fleet);
    };
    let Some(source_workspace) = profile
        .workflow
        .roots
        .value
        .as_ref()
        .and_then(|roots| roots.first())
        .map(PathBuf::from)
    else {
        return Ok(fleet);
    };
    ensure!(
        source_workspace.is_absolute()
            && source_workspace.is_dir()
            && !cloud_path(&source_workspace),
        "Isolated route workspace must be an existing local directory"
    );
    let snapshot_path = directory.join("snapshot.json");
    if !snapshot_path.exists() {
        return Ok(fleet);
    }
    let snapshot = crate::snapshot::load(directory)?;
    ensure!(
        snapshot.root.canonicalize()? == source_workspace.canonicalize()?,
        "The confirmed project root differs from this generation's snapshot"
    );
    let collaboration = crate::workspace::prepare(
        &crate::workspace::resolve(&crate::settings::load_settings())?.root,
    )?;
    ensure!(
        collaboration.root.canonicalize()? == source_workspace.canonicalize()?,
        "The approved collaboration workspace differs from the current selection"
    );
    let workspace = collaboration.root;
    let Some(portfolio) = table
        .portfolio
        .as_ref()
        .filter(|portfolio| portfolio.dispatch.executable)
    else {
        return Ok(fleet);
    };
    let mut indices = std::collections::BTreeSet::new();
    for role in portfolio
        .roles
        .iter()
        .filter(|role| role.seat != Seat::Orchestrator)
    {
        for rule in &role.rules {
            indices.extend(rule.row_index);
            indices.extend(rule.lower_effort.iter().map(|route| route.row_index));
            indices.extend(
                rule.within_provider_alternatives
                    .iter()
                    .map(|route| route.row_index),
            );
            indices.extend(
                rule.surplus_alternatives
                    .iter()
                    .map(|route| route.row_index),
            );
        }
    }
    for host in &profile.hosts {
        if let Some(binding) = host
            .active_binding_id
            .as_ref()
            .filter(|fact| fact.status != agent_setup::FactStatus::Unknown)
            .and_then(|fact| fact.value.as_ref())
        {
            indices.extend(
                table
                    .rows
                    .iter()
                    .enumerate()
                    .filter(|(_, row)| agent_setup::binding_id(row) == *binding)
                    .map(|(index, _)| index),
            );
        }
    }
    for index in indices {
        let row = table.rows.get(index).context("Route row is unavailable")?;
        let binding_id = agent_setup::binding_id(row);
        let Some(binding) = profile
            .bindings
            .iter()
            .find(|binding| binding.id == binding_id && binding.subscription_funded(false))
        else {
            continue;
        };
        let Some(host) = discovery
            .hosts
            .iter()
            .find(|host| host.id == binding.host_id && host.isolation_supported)
        else {
            continue;
        };
        let Some(model) = binding
            .native_model
            .value
            .as_ref()
            .filter(|model| !model.is_empty() && !model.starts_with('-'))
        else {
            continue;
        };
        let Some(effort_arguments) = binding.effort_args.value.as_ref() else {
            continue;
        };
        let Some(bundle) = profile
            .capability_bundles
            .iter()
            .find(|bundle| bundle.binding_id == binding.id)
        else {
            continue;
        };
        let effort = crate::isolation::mapped_effort(&host.kind, effort_arguments)?;
        fleet.routes.push(crate::isolation::Route {
            kind: host.kind,
            id: crate::team_policy::runtime_route_id(&binding_id, &bundle.launch_identity),
            binding_id,
            account_id: binding.account_id.clone(),
            executable: host.executable.clone().context("Isolated binary missing")?,
            model: model.clone(),
            effort,
            workspace: workspace.clone(),
            collaboration_root: workspace.clone(),
            home: directory.join("homes").join(&host.id),
            authenticated: profile
                .hosts
                .iter()
                .find(|recorded| recorded.id == host.id)
                .and_then(|recorded| recorded.isolation.as_ref())
                .filter(|fact| fact.status != agent_setup::FactStatus::Unknown)
                .and_then(|fact| fact.value.as_ref())
                .is_some_and(|confirmed| {
                    Path::new(&confirmed.generation_directory)
                        .canonicalize()
                        .ok()
                        == directory.canonicalize().ok()
                        && directory.exists()
                        && confirmed.account_ids.contains(&binding.account_id)
                        && Some(&confirmed.executable_identity) == host.isolation_identity.as_ref()
                        && Some(&confirmed.executable_version) == host.isolation_version.as_ref()
                }),
            executable_identity: host
                .isolation_identity
                .clone()
                .context("Native executable identity missing")?,
            permission: bundle.permission.clone(),
            network_scope: bundle.network_scope.clone(),
            source_scope: bundle.source_scope.clone(),
            exclusions: bundle.exclusions.clone(),
        });
    }
    let native_bindings: Vec<_> = profile
        .bindings
        .iter()
        .filter(|binding| {
            binding.subscription_funded(false)
                && profile
                    .capability_bundles
                    .iter()
                    .any(|bundle| bundle.binding_id == binding.id)
                && profile
                    .role_evidence
                    .iter()
                    .any(|evidence| evidence.binding_id == binding.id)
                && !fleet
                    .routes
                    .iter()
                    .any(|route| route.binding_id == binding.id)
        })
        .cloned()
        .collect();
    for binding in native_bindings {
        let Some(host) = discovery
            .hosts
            .iter()
            .find(|host| host.id == binding.host_id && host.isolation_supported)
        else {
            continue;
        };
        let Some(model) = binding
            .native_model
            .value
            .as_ref()
            .filter(|model| !model.is_empty() && !model.starts_with('-'))
        else {
            continue;
        };
        let Some(effort_arguments) = binding.effort_args.value.as_ref() else {
            continue;
        };
        let Some(bundle) = profile
            .capability_bundles
            .iter()
            .find(|bundle| bundle.binding_id == binding.id)
        else {
            continue;
        };
        let effort = crate::isolation::mapped_effort(&host.kind, effort_arguments)?;
        let authenticated = profile
            .hosts
            .iter()
            .find(|recorded| recorded.id == host.id)
            .and_then(|recorded| recorded.isolation.as_ref())
            .filter(|fact| fact.status != agent_setup::FactStatus::Unknown)
            .and_then(|fact| fact.value.as_ref())
            .is_some_and(|confirmed| {
                Path::new(&confirmed.generation_directory)
                    .canonicalize()
                    .ok()
                    == directory.canonicalize().ok()
                    && confirmed.account_ids.contains(&binding.account_id)
                    && Some(&confirmed.executable_identity) == host.isolation_identity.as_ref()
                    && Some(&confirmed.executable_version) == host.isolation_version.as_ref()
            });
        fleet.routes.push(crate::isolation::Route {
            kind: host.kind,
            id: crate::team_policy::runtime_route_id(&binding.id, &bundle.launch_identity),
            binding_id: binding.id.clone(),
            account_id: binding.account_id.clone(),
            executable: host.executable.clone().context("Isolated binary missing")?,
            model: model.clone(),
            effort,
            workspace: workspace.clone(),
            collaboration_root: workspace.clone(),
            home: directory.join("homes").join(&host.id),
            authenticated,
            executable_identity: host
                .isolation_identity
                .clone()
                .context("Native executable identity missing")?,
            permission: bundle.permission.clone(),
            network_scope: bundle.network_scope.clone(),
            source_scope: bundle.source_scope.clone(),
            exclusions: bundle.exclusions.clone(),
        });
    }
    if let Some(conductor) = &portfolio.conductor
        && !fleet
            .routes
            .iter()
            .any(|route| route.binding_id == conductor.binding_id)
        && let Some(binding) = profile.bindings.iter().find(|binding| {
            binding.id == conductor.binding_id
                && binding.provider_id == conductor.provider_id
                && binding.subscription_funded(false)
        })
        && let Some(host) = discovery
            .hosts
            .iter()
            .find(|host| host.id == binding.host_id && host.isolation_supported)
        && let Some(model) = binding
            .native_model
            .value
            .as_ref()
            .filter(|model| !model.is_empty() && !model.starts_with('-'))
        && let Some(effort_arguments) = binding.effort_args.value.as_ref()
    {
        let effort = crate::isolation::mapped_effort(&host.kind, effort_arguments)?;
        let Some(bundle) = profile
            .capability_bundles
            .iter()
            .find(|bundle| bundle.binding_id == binding.id)
        else {
            return Ok(fleet);
        };
        fleet.routes.push(crate::isolation::Route {
            kind: host.kind,
            id: "conductor".to_owned(),
            binding_id: conductor.binding_id.clone(),
            account_id: binding.account_id.clone(),
            executable: host.executable.clone().context("Isolated binary missing")?,
            model: model.clone(),
            effort,
            workspace: workspace.clone(),
            collaboration_root: workspace.clone(),
            home: directory.join("homes").join(&host.id),
            authenticated: profile
                .hosts
                .iter()
                .find(|recorded| recorded.id == host.id)
                .and_then(|recorded| recorded.isolation.as_ref())
                .filter(|fact| fact.status != agent_setup::FactStatus::Unknown)
                .and_then(|fact| fact.value.as_ref())
                .is_some_and(|confirmed| {
                    Path::new(&confirmed.generation_directory)
                        .canonicalize()
                        .ok()
                        == directory.canonicalize().ok()
                        && directory.exists()
                        && confirmed.account_ids.contains(&binding.account_id)
                        && Some(&confirmed.executable_identity) == host.isolation_identity.as_ref()
                        && Some(&confirmed.executable_version) == host.isolation_version.as_ref()
                }),
            executable_identity: host
                .isolation_identity
                .clone()
                .context("Native executable identity missing")?,
            permission: bundle.permission.clone(),
            network_scope: bundle.network_scope.clone(),
            source_scope: bundle.source_scope.clone(),
            exclusions: bundle.exclusions.clone(),
        });
    }
    let recommended = portfolio.conductor.as_ref();
    for host in &profile.hosts {
        let selected = recommended.and_then(|conductor| {
            fleet.routes.iter().find(|route| {
                route.id == "conductor"
                    && route.binding_id == conductor.binding_id
                    && profile
                        .bindings
                        .iter()
                        .any(|binding| binding.id == route.binding_id && binding.host_id == host.id)
            })
        });
        let override_binding = profile
            .orchestrator_host_override
            .value
            .as_ref()
            .filter(|confirmed| {
                confirmed.host_id == host.id
                    && profile.orchestrator_host_override.status != agent_setup::FactStatus::Unknown
                    && recommended.is_some_and(|conductor| {
                        conductor.provider_id == confirmed.recommended_provider
                    })
            })
            .and(host.active_binding_id.as_ref())
            .filter(|fact| fact.status != agent_setup::FactStatus::Unknown)
            .and_then(|fact| fact.value.as_ref());
        let selected = selected.or_else(|| {
            override_binding.and_then(|id| {
                let expected = recommended;
                fleet.routes.iter().find(|route| {
                    route.id == "conductor"
                        && route.binding_id == *id
                        && profile.bindings.iter().any(|binding| {
                            binding.id == route.binding_id && binding.host_id == host.id
                        })
                        && expected.is_some_and(|conductor| {
                            route.binding_id == conductor.binding_id
                                && route.account_id
                                    == profile
                                        .providers
                                        .get(&conductor.provider_id)
                                        .map(|provider| provider.account_id.clone())
                                        .unwrap_or_default()
                        })
                })
            })
        });
        let selected = selected.or_else(|| {
            let mut candidates: Vec<_> = fleet
                .routes
                .iter()
                .filter(|route| {
                    route.authenticated
                        && profile.bindings.iter().any(|binding| {
                            binding.id == route.binding_id && binding.host_id == host.id
                        })
                        && profile.role_evidence.iter().any(|evidence| {
                            evidence.binding_id == route.binding_id
                                && evidence.seat == Seat::Orchestrator
                        })
                })
                .collect();
            candidates.sort_by(|left, right| left.id.cmp(&right.id));
            candidates.into_iter().next()
        });
        if let Some(route) = selected.filter(|route| route.authenticated) {
            fleet.conductors.insert(host.id.clone(), route.id.clone());
        }
    }
    Ok(fleet)
}

fn policy_files(table: &Table, directory: &Path) -> Result<Vec<StagedFile>> {
    let mut files = Vec::new();
    let mut index = Vec::new();
    if let Some(portfolio) = &table.portfolio {
        if let Some(conductor) = &portfolio.conductor {
            let name = "policies/orchestrator.json";
            let row = table
                .rows
                .get(conductor.row_index)
                .context("Conductor benchmark row unavailable")?;
            let value = serde_json::json!({"role":"Orchestrator","scope":"whole_plan","route":"conductor","binding_id":conductor.binding_id,"benchmark":{"harness":row.harness,"model":row.model,"effort":row.effort},"native_harness":conductor.native_harness,"provider":conductor.provider_id,"plan":conductor.plan_id,"attempt_limit":conductor.attempt_limit,"competence":conductor.competence,"utility":conductor.utility,"expected_visits":conductor.expected_visits,"reserved_visits":conductor.reserved_visits,"expected_usage":conductor.expected_usage,"expected_hours":conductor.expected_hours,"reserved_usage":conductor.reserved_usage,"reserved_hours":conductor.reserved_hours,"per_call_expected_usage":conductor.per_call_expected_usage,"per_call_expected_hours":conductor.per_call_expected_hours,"per_call_reserved_usage":conductor.per_call_reserved_usage,"per_call_reserved_hours":conductor.per_call_reserved_hours,"cost_basis":conductor.cost_basis,"time_basis":conductor.time_basis,"classes":conductor.classes});
            index.push(serde_json::json!({"role":"Orchestrator","scope":"whole_plan","file":name}));
            files.push(StagedFile {
                path: directory.join(name),
                root: directory.to_path_buf(),
                bytes: serde_json::to_vec_pretty(&value)?,
                executable: false,
            });
        }
        for role in portfolio
            .roles
            .iter()
            .filter(|role| role.seat != Seat::Orchestrator)
        {
            for rule in &role.rules {
                let name = format!(
                    "policies/{}-{}.json",
                    role.seat.name().to_ascii_lowercase().replace(' ', "-"),
                    rule.class.name().to_ascii_lowercase()
                );
                let adaptive = |route: &crate::portfolio::AdaptiveRoute| serde_json::json!({"route":crate::team_policy::route_id(&route.binding_id),"binding_id":route.binding_id,"attempt_limit":route.attempt_limit,"utility":route.quality,"competence":route.competence,"expected_usage":route.per_call_expected_usage,"expected_hours":route.per_call_expected_hours,"reserved_usage":route.per_call_reserved_usage,"reserved_hours":route.per_call_reserved_hours,"provider":route.provider_id,"opportunity_cost":route.opportunity_cost});
                let primary_route = rule
                    .row_index
                    .and_then(|index| table.rows.get(index))
                    .map(|row| {
                        rule.research_candidates
                            .iter()
                            .find(|candidate| {
                                candidate.primary
                                    && candidate.row_index == rule.row_index.unwrap_or_default()
                            })
                            .map_or_else(
                                || crate::agent_setup::binding_id(row),
                                |candidate| {
                                    crate::agent_setup::binding_id_for(
                                        &candidate.native_harness,
                                        row,
                                    )
                                },
                            )
                    })
                    .map(|binding| crate::team_policy::route_id(&binding));
                let research_candidates: Vec<_> = rule.research_candidates.iter().filter_map(|candidate| table.rows.get(candidate.row_index).map(|row| serde_json::json!({"route":crate::team_policy::route_id(&candidate.binding_id),"binding_id":candidate.binding_id,"primary":candidate.primary,"benchmark":{"harness":row.harness,"model":row.model,"effort":row.effort,"source":candidate.source},"native_harness":candidate.native_harness,"provider":candidate.provider_id,"plan":candidate.plan_id,"score":candidate.score,"components":{"omniscience_accuracy":{"value":candidate.accuracy,"weight":candidate.accuracy_weight},"omniscience_no_incorrect_answer":{"value":candidate.non_wrong,"weight":candidate.non_wrong_weight},"aa_lcr":{"value":candidate.lcr,"weight":candidate.lcr_weight},"hle":{"value":candidate.hle,"weight":candidate.hle_weight}},"diagnostics":{"omniscience_conditional_hallucination":candidate.conditional_hallucination,"gpqa_scientific":candidate.gpqa_diagnostic,"gdp_pdf_document":candidate.gdp_pdf_diagnostic},"api_proxy":{"expected_usd":candidate.expected_usd,"decode_hours":candidate.decode_hours},"eligibility":candidate.eligibility}))).collect();
                let research_included = portfolio
                    .dispatch
                    .classes
                    .iter()
                    .find(|demand| demand.class == rule.class)
                    .is_some_and(|demand| demand.research_included);
                let value = serde_json::json!({"policy_version":crate::team_policy::VERSION,"role":role.seat.name(),"class":rule.class.name(),"condition":rule.condition,"research_reference_included":research_included,"primary":{"route":primary_route,"recommendation_only":rule.recommendation_only,"attempt_limit":rule.attempt_limit,"utility":rule.utility,"competence":rule.competence,"weekly_expected_usage":rule.nominal_usage,"weekly_expected_hours":rule.nominal_hours,"weekly_reserved_usage":rule.reserved_usage,"weekly_reserved_hours":rule.reserved_hours,"expected_usage":rule.per_call_expected_usage,"expected_hours":rule.per_call_expected_hours,"per_call_reserved_usage":rule.per_call_reserved_usage,"per_call_reserved_hours":rule.per_call_reserved_hours,"provider":rule.provider_id},"research_candidates":research_candidates,"lower_effort":rule.lower_effort.as_ref().map(adaptive),"same_account":rule.within_provider_alternatives.iter().map(adaptive).collect::<Vec<_>>(),"other_accounts":rule.surplus_alternatives.iter().map(adaptive).collect::<Vec<_>>(),"fallback":rule.fallback_policy,"calibration":rule.calibration,"failure_action":rule.failure_action});
                index.push(serde_json::json!({"role":role.seat.name(),"class":rule.class.name(),"file":name}));
                files.push(StagedFile {
                    path: directory.join(&name),
                    root: directory.to_path_buf(),
                    bytes: serde_json::to_vec_pretty(&value)?,
                    executable: false,
                });
            }
        }
    }
    files.push(StagedFile {
        path: directory.join("policy-index.json"),
        root: directory.to_path_buf(),
        bytes: serde_json::to_vec_pretty(&index)?,
        executable: false,
    });
    Ok(files)
}

pub(crate) fn required_bindings(table: &Table) -> Vec<serde_json::Value> {
    let mut indices = std::collections::BTreeSet::new();
    if let Some(portfolio) = &table.portfolio {
        for role in portfolio
            .roles
            .iter()
            .filter(|role| role.seat != Seat::Orchestrator)
        {
            for rule in &role.rules {
                indices.extend(rule.row_index);
                indices.extend(
                    rule.lower_effort
                        .iter()
                        .chain(&rule.within_provider_alternatives)
                        .chain(&rule.surplus_alternatives)
                        .map(|route| route.row_index),
                );
            }
        }
    }
    let mut bindings: Vec<_> = indices.into_iter().filter_map(|index|table.rows.get(index).map(|row|{let binding=agent_setup::binding_id(row);serde_json::json!({"route":crate::team_policy::route_id(&binding),"binding_id":binding,"model":row.model,"effort":row.effort,"harness":row.harness,"provider":row.vendor})})).collect();
    if let Some(portfolio) = &table.portfolio {
        let mut research: std::collections::BTreeSet<_> = bindings
            .iter()
            .filter_map(|binding| binding["binding_id"].as_str().map(str::to_owned))
            .collect();
        for candidate in portfolio
            .roles
            .iter()
            .flat_map(|role| &role.rules)
            .flat_map(|rule| &rule.research_candidates)
        {
            if research.insert(candidate.binding_id.clone())
                && let Some(row) = table.rows.get(candidate.row_index)
            {
                bindings.push(serde_json::json!({"route":crate::team_policy::route_id(&candidate.binding_id),"binding_id":candidate.binding_id,"model":row.model,"effort":row.effort,"harness":candidate.native_harness,"benchmark_harness":row.harness,"provider":candidate.provider_id,"plan":candidate.plan_id,"conditional_role":"Net Research"}));
            }
        }
    }
    if let Some(conductor) = table
        .portfolio
        .as_ref()
        .and_then(|portfolio| portfolio.conductor.as_ref())
        && let Some(row) = table.rows.get(conductor.row_index)
    {
        bindings.push(serde_json::json!({"route":"conductor","binding_id":conductor.binding_id,"model":row.model,"effort":row.effort,"harness":conductor.native_harness,"benchmark_harness":row.harness,"provider":conductor.provider_id}));
    }
    bindings
}

pub fn prepare_retry(
    discovery: &Discovery,
    host_ids: &[String],
    installed: &InstallResult,
) -> Result<InstallPreview> {
    ensure!(
        host_ids.iter().all(|id| installed
            .failures
            .iter()
            .any(|failure| &failure.host_id == id)
            && !installed
                .launchers
                .iter()
                .any(|launcher| &launcher.host_id == id)),
        "Retry may only select failed hosts from this installation"
    );
    ensure!(
        executable(installed.policy_path.clone()).is_some(),
        "The existing canonical orchestrator Markdown is unavailable"
    );
    prepare(
        "",
        discovery,
        host_ids,
        false,
        Some(installed),
        None,
        installed.isolated,
    )
}

fn prepare(
    markdown: &str,
    discovery: &Discovery,
    host_ids: &[String],
    add_to_user_path: bool,
    installed: Option<&InstallResult>,
    table: Option<&Table>,
    isolated: bool,
) -> Result<InstallPreview> {
    ensure!(
        installed.is_some() || !markdown.trim().is_empty(),
        "The orchestrator Markdown is empty"
    );
    let generation = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos()
        .to_string();
    let directory = installed
        .map(|result| result.directory.clone())
        .unwrap_or_else(|| discovery.paths.generations.join(&generation));
    let policy_path = installed
        .map(|result| result.policy_path.clone())
        .unwrap_or_else(|| directory.join("orchestrator.md"));
    destination_allowed(&policy_path, &directory)?;
    let app = executable(std::env::current_exe()?)
        .context("The application executable is unavailable or cloud-backed")?;
    let home = directories::BaseDirs::new()
        .context("Cannot locate the user directory")?
        .home_dir()
        .to_path_buf();
    let mut staged = Vec::new();
    let mut destinations = if installed.is_some() {
        Vec::new()
    } else {
        vec![policy_path.clone()]
    };
    let mut commands = Vec::new();
    let mut host_impacts = Vec::new();
    let mut selected = Vec::new();
    for id in host_ids {
        ensure!(
            safe_host_id(id),
            "Host ID must contain only lowercase letters, digits and hyphens, beginning with a letter (maximum 96 characters)"
        );
        ensure!(!selected.contains(id), "Host selected more than once: {id}");
        let host = discovery
            .hosts
            .iter()
            .find(|host| &host.id == id)
            .context("Selected host is no longer in this discovery snapshot")?;
        let binary = host
            .executable
            .as_ref()
            .context("Selected host has no installed executable")?;
        ensure!(
            host.installed && executable(binary.clone()).is_some() && !known_desktop(binary),
            "Host executable is unavailable: {}",
            binary.display()
        );
        let directive = read_directive(&policy_path);
        let mut files = Vec::new();
        ensure!(
            host.environment
                .iter()
                .all(|(name, value)| allowed_environment(name)
                    && Path::new(value).is_absolute()
                    && !cloud_path(Path::new(value))),
            "Unsupported host environment override"
        );
        ensure!(
            safe_arguments(&host.arguments),
            "Host arguments contain a permission-bypass option"
        );
        ensure!(
            !isolated || host.isolation_supported,
            "Selected host does not support verified isolation"
        );
        let mut arguments = if isolated {
            Vec::new()
        } else {
            host.arguments.clone()
        };
        let mut adapter = match host.kind {
            HostKind::OpenCode => {
                arguments.extend(["--prompt".to_owned(), directive.clone()]);
                "Interactive OpenCode prompt referencing canonical Markdown"
            }
            HostKind::Aider => {
                arguments.extend([
                    "--read".to_owned(),
                    policy_path.to_string_lossy().into_owned(),
                ]);
                "Interactive Aider with read-only canonical Markdown context; waits for user input"
            }
            HostKind::Goose => {
                arguments.extend([
                    "run".to_owned(),
                    "--instructions".to_owned(),
                    policy_path.to_string_lossy().into_owned(),
                    "--interactive".to_owned(),
                ]);
                "Interactive Goose with canonical Markdown instructions"
            }
            HostKind::Cline => {
                arguments.extend(["-i".to_owned(), directive.clone()]);
                "Interactive Cline TUI seeded with the canonical-file directive"
            }
            HostKind::OpenHands => {
                arguments.extend(["-f".to_owned(), policy_path.to_string_lossy().into_owned()]);
                "Interactive OpenHands with canonical Markdown instructions"
            }
            HostKind::Pi => {
                arguments.extend([
                    "--".to_owned(),
                    format!("@{}", policy_path.display()),
                    directive.clone(),
                ]);
                "Interactive Pi with canonical Markdown attached as context"
            }
            HostKind::Droid => {
                arguments.extend([
                    "--append-system-prompt-file".to_owned(),
                    policy_path.to_string_lossy().into_owned(),
                    directive.clone(),
                ]);
                "Interactive Droid with additive canonical Markdown instructions"
            }
            HostKind::Buzz => {
                bail!("Buzz Desktop has no verified interactive orchestrator CLI adapter")
            }
            _ => {
                arguments.push(directive.clone());
                "Interactive canonical-file prompt"
            }
        }
        .to_owned();
        let digest = Sha256::digest(format!("{generation}:{}", host.id));
        let suffix = digest[..12]
            .iter()
            .flat_map(|byte| {
                [
                    char::from(b'a' + (byte >> 4)),
                    char::from(b'a' + (byte & 15)),
                ]
            })
            .collect::<String>();
        let agent_name = format!("aitierlist-orchestrator-{suffix}");
        if host.native_adapter && !isolated {
            let target = match host.kind {
                HostKind::Claude => {
                    Some(home.join(".claude/agents").join(format!("{agent_name}.md")))
                }
                HostKind::Antigravity => Some(
                    home.join(".gemini/config/agents")
                        .join(&agent_name)
                        .join("agent.md"),
                ),
                _ => None,
            };
            if let Some(target) = target {
                let primary = if matches!(host.kind, HostKind::Antigravity) {
                    "mainAgent: true\n"
                } else {
                    ""
                };
                let body = format!(
                    "---\nname: {agent_name}\ndescription: Load the canonical aitierlist orchestrator and perform required onboarding before work.\n{primary}---\n\n{directive}\n"
                );
                files.push(StagedFile {
                    root: match host.kind {
                        HostKind::Claude => home.join(".claude/agents"),
                        _ => home.join(".gemini/config/agents"),
                    },
                    path: target,
                    bytes: body.into_bytes(),
                    executable: false,
                });
                arguments = host.arguments.clone();
                arguments.extend(["--agent".to_string(), agent_name]);
                adapter = "Native primary agent referencing canonical Markdown".to_string();
            }
        }
        let stem = format!("aitierlist-orchestrator-{}", host.id);
        let launcher_path = unused_path(
            &discovery.paths.bin,
            &stem,
            if cfg!(windows) { ".cmd" } else { "" },
        )?;
        let manifest_path = unused_path(&directory, &host.id, ".json")?;
        let isolated_home = isolated.then(|| directory.join("homes").join(&host.id));
        if isolated {
            arguments = crate::isolation::conductor_arguments(
                &host.kind,
                &directory.join("bootstrap"),
                &discovery.paths.directory,
                format!(
                    "Read {} and follow its coordinator protocol. Load onboarding schemas only when setup is incomplete. Use only the fixed fleet commands for workers.",
                    directory.join("runtime.md").display()
                ),
            )?;
            adapter = "Isolated native defaults with fixed fleet dispatcher".to_owned();
        }
        let manifest = LaunchManifest {
            version: 1,
            executable: binary.clone(),
            arguments,
            environment: if isolated {
                BTreeMap::new()
            } else {
                host.environment.clone()
            },
            policy_path: policy_path.clone(),
            isolated_home,
            workspace: isolated.then(|| directory.join("bootstrap")),
            kind: isolated.then_some(host.kind),
            isolation_identity: isolated.then(|| host.isolation_identity.clone()).flatten(),
            host_id: isolated.then(|| host.id.clone()),
        };
        host_impacts.push(format!(
            "{}: executable {}; arguments {}; config-home overrides {}",
            host.name,
            binary.display(),
            manifest
                .arguments
                .iter()
                .map(|argument| powershell_quote(argument))
                .collect::<Vec<_>>()
                .join(" "),
            if isolated {
                format!(
                    "clean generation home {}; inherited customization overrides removed",
                    directory.join("homes").join(&host.id).display()
                )
            } else if manifest.environment.is_empty() {
                "none; inherit process environment".to_string()
            } else {
                manifest
                    .environment
                    .iter()
                    .map(|(name, value)| format!("{name}={value}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
        files.push(StagedFile {
            root: directory.clone(),
            path: manifest_path.clone(),
            bytes: serde_json::to_vec_pretty(&manifest)?,
            executable: false,
        });
        let launcher = if cfg!(windows) {
            let script = format!(
                "& {} '--launch-orchestrator' {}",
                powershell_quote(&app.to_string_lossy()),
                powershell_quote(&manifest_path.to_string_lossy())
            );
            format!(
                "@echo off\r\nsetlocal DisableDelayedExpansion\r\n\"%SystemRoot%\\System32\\WindowsPowerShell\\v1.0\\powershell.exe\" -NoLogo -NoProfile -EncodedCommand {}\r\n",
                encoded_powershell(&script)
            )
        } else {
            format!(
                "#!/bin/sh\nexec {} --launch-orchestrator {}\n",
                shell_quote(&app.to_string_lossy()),
                shell_quote(&manifest_path.to_string_lossy())
            )
        };
        files.push(StagedFile {
            root: discovery.paths.bin.clone(),
            path: launcher_path.clone(),
            bytes: launcher.into_bytes(),
            executable: true,
        });
        for file in &files {
            destination_allowed(&file.path, &file.root)?;
            ensure!(
                !cloud_path(&file.path),
                "Installation destination is cloud-backed: {}",
                file.path.display()
            );
            ensure!(
                !file.path.try_exists()?,
                "Installation would overwrite {}",
                file.path.display()
            );
            destinations.push(file.path.clone());
        }
        let command = if cfg!(windows) {
            format!("& {}", powershell_quote(&launcher_path.to_string_lossy()))
        } else {
            shell_quote(&launcher_path.to_string_lossy())
        };
        if add_to_user_path {
            commands.push(format!(
                "{} (new terminal); current terminal: {command}",
                launcher_path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
        } else {
            commands.push(command.clone());
        }
        staged.push(StagedHost {
            files,
            launcher: InstalledLauncher {
                host_id: id.clone(),
                adapter,
                path: launcher_path,
                command,
            },
        });
        selected.push(id.clone());
    }
    ensure!(
        !cloud_path(&directory) && !cloud_path(&discovery.paths.bin),
        "Installation directory is cloud-backed"
    );
    let mut impacts = vec![format!("Create one editable canonical Markdown file: {}", policy_path.display()), "Create only the listed new files. Existing files are never overwritten; collisions fail for the affected host.".to_string(), "Each launcher opens an interactive host in the caller's current directory. Model, effort, approvals, and sandbox flags are not changed.".to_string(), "Host definitions load the canonical Markdown on every new launch; editing that Markdown updates all launchers in this installation.".to_string()];
    if isolated {
        impacts[0] = format!(
            "Create isolated runtime instructions and compact policies in {}; the standalone orchestrator.md is a separate portable export.",
            directory.display()
        );
        impacts[2]="Isolated launchers use a clean bootstrap, then the independent project copy. After onboarding, approved conductor and worker model/effort bindings are compiled by the app; native approvals, network restrictions and managed policy remain. Codex workers use the native :workspace permission profile for the isolated copy and native temporary paths; its conductor additionally receives the app-owned orchestrator directory for profiles, ledger and job files.".to_owned();
        impacts[3]="Finalize creates the isolated project copy and records exact exclusions. Review changes and explicitly approve apply-back to the original project. Fresh sign-in may be required. Editing portable orchestrator.md does not change compiled runtime routes.".to_owned();
    }
    if installed.is_some() {
        impacts[0] = format!(
            "Reuse the existing canonical Markdown unchanged: {}. Successful hosts and common schema files are not rewritten.",
            policy_path.display()
        );
    }
    impacts.extend(host_impacts);
    impacts.push(format!("Launchers depend on this application executable remaining at {}. Moving or deleting it requires reinstalling the launchers.", app.display()));
    if add_to_user_path && !selected.is_empty() {
        ensure!(
            !discovery.paths.bin.to_string_lossy().contains('%'),
            "User PATH setup is unavailable for a directory containing %. Use the absolute launcher command instead."
        );
        impacts.push(format!(
            "Append {} to the current user's PATH. Open a new terminal afterward.",
            discovery.paths.bin.display()
        ));
        ensure!(
            cfg!(windows),
            "Automatic user PATH setup is only supported on Windows; use the absolute launcher path on this platform"
        );
    }
    let add_to_user_path = add_to_user_path && !selected.is_empty();
    let mut common_files = if installed.is_some() {
        Vec::new()
    } else {
        vec![
            StagedFile {
                root: directory.clone(),
                path: policy_path.clone(),
                bytes: markdown.as_bytes().to_vec(),
                executable: false,
            },
            StagedFile {
                root: directory.clone(),
                path: directory.join("onboarding.schema.json"),
                bytes: agent_setup::setup_schema().into_bytes(),
                executable: false,
            },
            StagedFile {
                root: directory.clone(),
                path: directory.join("ledger.schema.json"),
                bytes: agent_setup::ledger_schema().into_bytes(),
                executable: false,
            },
        ]
    };
    if isolated && installed.is_none() {
        let fleet = isolated_fleet(
            discovery,
            table.context("Isolated generation requires a table snapshot")?,
            &directory,
        )?;
        let fleet_path = directory.join("fleet.json");
        let command = "fleet";
        let mut runtime_paths = discovery.paths.clone();
        runtime_paths.profile_schema = directory.join("onboarding.schema.json");
        runtime_paths.ledger_schema = directory.join("ledger.schema.json");
        let runtime = format!(
            "{}\n\n## Fixed fleet commands\n\nInspect setup metadata: {command} inspect setup\nFinalize onboarding and project copy: {command} finalize setup\nSign in: {command} login <host-id>\nAdmit: {command} admit task <task-json-path>\nReplan: {command} replan queue\nUpdate unstarted scope or risk: {command} update <task-id> <task-json-path>\nRun assigned work: {command} run <visit-id>\nSettle: {command} settle <visit-id> <settlement-json-path>\nReconcile a native meter: {command} reconcile meter <observation-json-path>\nStatus: {command} status <task-or-visit-id>\nCancel: {command} cancel <task-id>\nReview changes: {command} diff snapshot\nApply reviewed changes: {command} apply snapshot\n\nTask input carries an authorized generation and workspace identity, project and idempotency identity, risk vector, evidence and resource requirements, dependencies, priority, deadline, value, human demand, brief path, and timeout. Admission creates a queue-wide native horizon. The ledger horizon pointer and its immutable artifact are authoritative. Run accepts only a visit ID from that committed horizon. Read result.json for process output, then settle observed native usage and artifact disposition explicitly. Process completion does not settle usage or accept an artifact.\n\nCodex workers use the native :workspace permission profile for the isolated project copy. The conductor additionally receives the app-owned orchestrator directory. Native permissions, network restrictions, source scope, account binding, model, effort, executable identity, and launch recipe must match the verified capability bundle. Built-in worker delegation is disabled for Codex and Grok; every worker receives one seat-specific brief and immutable dependency artifacts.\n",
            crate::orchestration::isolated_protocol(
                table.context("Missing generation table")?,
                &runtime_paths
            )
        );
        for (name, bytes) in [
            ("fleet.json", serde_json::to_vec_pretty(&fleet)?),
            ("runtime.md", runtime.into_bytes()),
            (
                "discovery.json",
                serde_json::to_vec(&DiscoverySeed {
                    paths: discovery.paths.clone(),
                    hosts: discovery.hosts.clone(),
                })?,
            ),
            (
                "table.json",
                serde_json::to_vec(table.context("Missing generation table")?)?,
            ),
        ] {
            common_files.push(StagedFile {
                path: directory.join(name),
                root: directory.clone(),
                bytes,
                executable: false,
            });
        }
        let launcher = format!(
            "@echo off\r\nsetlocal DisableDelayedExpansion\r\n\"{}\" --fleet \"{}\" %*\r\nexit /b %errorlevel%\r\n",
            app.to_string_lossy()
                .replace("\\\\?\\", "")
                .replace('%', "%%"),
            fleet_path
                .to_string_lossy()
                .replace("\\\\?\\", "")
                .replace('%', "%%")
        );
        common_files.push(StagedFile {
            path: directory.join("bin/fleet.cmd"),
            root: directory.clone(),
            bytes: launcher.into_bytes(),
            executable: false,
        });
        impacts.push("Isolation uses native safe mode and fresh generation homes; unsupported workers are omitted. Shared quota reservations remain the explicit existing ledger protocol. Fleet commands fix all native invocation details.".to_owned());
        common_files.extend(policy_files(
            table.context("Missing generation table")?,
            &directory,
        )?);
    }
    destinations.extend(common_files.iter().skip(1).map(|file| file.path.clone()));
    Ok(InstallPreview {
        isolated,
        directory,
        policy_path: policy_path.clone(),
        destinations,
        impacts,
        commands,
        host_ids: selected,
        add_to_user_path,
        common_files,
        hosts: staged,
        bin: discovery.paths.bin.clone(),
    })
}

fn write_new(file: &StagedFile) -> Result<()> {
    destination_allowed(&file.path, &file.root)?;
    let parent = file
        .path
        .parent()
        .context("Installation file has no parent")?;
    ensure!(
        !cloud_path(parent),
        "Cloud-backed installation destination is not allowed"
    );
    if let Some(existing) = parent.ancestors().find(|path| path.exists()) {
        ensure!(
            !cloud_path(&existing.canonicalize()?),
            "Installation destination resolves into a cloud-backed directory"
        );
    }
    fs::create_dir_all(parent).with_context(|| format!("Create {}", parent.display()))?;
    ensure!(
        !cloud_path(&parent.canonicalize()?),
        "Installation destination resolves into a cloud-backed directory"
    );
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&file.path)
        .with_context(|| format!("Create {} without overwriting", file.path.display()))?;
    output.write_all(&file.bytes)?;
    output.sync_all()?;
    #[cfg(unix)]
    if file.executable {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&file.path, fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = file.executable;
    Ok(())
}

fn add_user_path(bin: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let system =
            PathBuf::from(std::env::var_os("SystemRoot").context("SystemRoot is unavailable")?);
        let powershell = system.join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let value = bin.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$ErrorActionPreference='Stop'; $key=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment'); try {{ $old=[string]$key.GetValue('Path','',[Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); $entry='{value}'; if (-not (($old -split ';') | Where-Object {{ $_.TrimEnd('\\') -ieq $entry.TrimEnd('\\') }})) {{ $kind=if($key.GetValueNames() -contains 'Path'){{$key.GetValueKind('Path')}}else{{[Microsoft.Win32.RegistryValueKind]::ExpandString}}; $next=if([string]::IsNullOrEmpty($old)){{$entry}}else{{$old.TrimEnd(';')+';'+$entry}}; $key.SetValue('Path',$next,$kind) }} }} finally {{ $key.Dispose() }}"
        );
        let output = Command::new(powershell)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &script,
            ])
            .creation_flags(0x08000000)
            .output()?;
        ensure!(
            output.status.success(),
            "User PATH update failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        #[link(name = "user32")]
        unsafe extern "system" {
            fn SendMessageTimeoutW(
                window: *mut std::ffi::c_void,
                message: u32,
                wparam: usize,
                lparam: isize,
                flags: u32,
                timeout: u32,
                result: *mut usize,
            ) -> isize;
        }
        let environment = "Environment\0".encode_utf16().collect::<Vec<_>>();
        unsafe {
            SendMessageTimeoutW(
                0xffffusize as *mut std::ffi::c_void,
                0x001a,
                0,
                environment.as_ptr() as isize,
                0x0002,
                3000,
                std::ptr::null_mut(),
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = bin;
        bail!("Automatic user PATH setup is unavailable on this platform")
    }
}

pub fn install(preview: InstallPreview) -> Result<InstallResult> {
    preview.common_files.iter().try_for_each(write_new)?;
    let mut result = InstallResult {
        isolated: preview.isolated,
        directory: preview.directory,
        policy_path: preview.policy_path,
        launchers: Vec::new(),
        failures: Vec::new(),
        path_added: None,
        notes: Vec::new(),
    };
    for host in preview.hosts {
        let outcome = host.files.iter().try_for_each(write_new);
        match outcome {
            Ok(()) => result.launchers.push(host.launcher),
            Err(error) => {
                let message = format!(
                    "{error:#}. Some listed files may have been created; existing files were not overwritten."
                );
                result
                    .notes
                    .push(format!("{}: {message}", host.launcher.host_id));
                result.failures.push(InstallFailure {
                    host_id: host.launcher.host_id,
                    message,
                });
            }
        }
    }
    if preview.add_to_user_path && !result.launchers.is_empty() {
        match add_user_path(&preview.bin) {
            Ok(()) => {
                result.path_added = Some(preview.bin);
                result.notes.push("User PATH updated. Start a new terminal to use launcher names; absolute paths work immediately.".to_string());
            }
            Err(error) => result.notes.push(format!(
                "Launchers installed, but user PATH was not updated: {error:#}"
            )),
        }
    }
    Ok(result)
}

pub fn launch(manifest_path: &Path) -> Result<()> {
    ensure!(
        executable(manifest_path.to_path_buf()).is_some(),
        "Launcher manifest is unavailable or cloud-backed"
    );
    let mut bytes = Vec::new();
    fs::File::open(manifest_path)?
        .take(65537)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 65536,
        "Launcher manifest exceeds the supported size"
    );
    let manifest: LaunchManifest = serde_json::from_slice(&bytes)?;
    ensure!(
        manifest.version == 1,
        "Unsupported launcher manifest version"
    );
    ensure!(
        manifest.executable.is_absolute() && manifest.policy_path.is_absolute(),
        "Launcher paths must be absolute"
    );
    ensure!(
        manifest
            .environment
            .iter()
            .all(|(name, value)| allowed_environment(name)
                && Path::new(value).is_absolute()
                && !cloud_path(Path::new(value))),
        "Unsupported launcher environment override"
    );
    ensure!(
        safe_arguments(&manifest.arguments),
        "Launcher contains permission-bypass arguments"
    );
    ensure!(
        manifest
            .arguments
            .iter()
            .map(|value| value.encode_utf16().count() + 3)
            .sum::<usize>()
            < 30000,
        "Launcher arguments exceed the supported command length"
    );
    ensure!(
        executable(manifest.executable.clone()).is_some() && !known_desktop(&manifest.executable),
        "Host executable is no longer available: {}",
        manifest.executable.display()
    );
    ensure!(
        executable(manifest.policy_path.clone()).is_some(),
        "Canonical orchestrator Markdown is unavailable"
    );
    let mut command = if let Some(home) = &manifest.isolated_home {
        let directory = manifest_path.parent().context("Manifest parent missing")?;
        destination_allowed(home, directory)?;
        ensure!(
            manifest
                .workspace
                .as_deref()
                .map(resolved_future)
                .transpose()?
                == Some(resolved_future(&directory.join("bootstrap"))?),
            "Isolated conductor workspace is not its bound bootstrap"
        );
        ensure!(
            Some(crate::isolation::executable_identity(&manifest.executable)?)
                == manifest.isolation_identity,
            "Native executable changed; repeat discovery and installation"
        );
        crate::isolation::prepare_home(home, directory)?;
        crate::isolation::bootstrap(
            manifest
                .workspace
                .as_deref()
                .context("Isolated workspace missing")?,
            home,
            &git_binary()?,
        )?;
        crate::isolation::clean_command(
            &manifest.executable,
            home,
            manifest
                .workspace
                .as_deref()
                .context("Isolated workspace missing")?,
        )
    } else {
        Command::new(&manifest.executable)
    };
    let mut conductor_reservation = None;
    if manifest.isolated_home.is_some() {
        let directory = manifest_path.parent().context("Manifest parent missing")?;
        let route = crate::isolation::conductor_route(
            &directory.join("fleet.json"),
            manifest
                .host_id
                .as_deref()
                .context("Isolated host ID missing")?,
        )?;
        let workspace = if let Some(route) = &route {
            let collaboration = crate::workspace::resolve_root(&route.collaboration_root)?;
            ensure!(
                route.executable.canonicalize()? == manifest.executable.canonicalize()?
                    && Some(&route.kind) == manifest.kind.as_ref()
                    && route.workspace.canonicalize()? == collaboration.root.canonicalize()?,
                "Approved conductor binding differs from installed host"
            );
            conductor_reservation = Some(crate::isolation::reserve_conductor_launch(
                &directory.join("fleet.json"),
                route,
            )?);
            command.args(crate::isolation::model_arguments(
                &route.kind,
                &route.model,
                route.effort.as_deref(),
            )?);
            route.workspace.clone()
        } else {
            directory.join("bootstrap")
        };
        command.current_dir(&workspace);
        let seed: DiscoverySeed =
            serde_json::from_reader(fs::File::open(directory.join("discovery.json"))?)?;
        destination_allowed(directory, &seed.paths.generations)?;
        command.args(crate::isolation::conductor_arguments(manifest.kind.as_ref().context("Isolated host kind missing")?, &workspace, &seed.paths.directory, format!("Read {} and follow its coordinator protocol. Use fixed fleet commands only; load schemas only for onboarding.",directory.join("runtime.md").display()))?);
    } else {
        command
            .args(&manifest.arguments)
            .envs(&manifest.environment);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000010);
    }
    let child = match command
        .spawn()
        .with_context(|| format!("Launch {}", manifest.executable.display()))
    {
        Ok(child) => child,
        Err(error) => {
            if let Some(reservation) = conductor_reservation {
                crate::isolation::conductor_launch_failed(reservation)?;
            }
            return Err(error);
        }
    };
    if let Some(reservation) = conductor_reservation {
        crate::isolation::conductor_started(reservation, &child)?;
    }
    Ok(())
}
