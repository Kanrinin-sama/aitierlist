use crate::file_tx::{self, DirectoryLease, OwnedStagingFile};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub id: String,
    pub root: PathBuf,
    pub shadow: PathBuf,
    pub generation: PathBuf,
    pub data: PathBuf,
    pub git: PathBuf,
    pub preimages: BTreeMap<String, Option<String>>,
    pub excluded: Vec<String>,
    pub git_config_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    pub kind: String,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub workspace_id: String,
    pub root: PathBuf,
    pub shadow: PathBuf,
    pub changes: Vec<Change>,
    pub report: PathBuf,
    pub patch: PathBuf,
}

#[derive(Debug, Serialize)]
pub struct ApplyReport {
    pub applied: bool,
    pub files: Vec<String>,
    pub message: String,
    pub report: PathBuf,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn cloud(path: &Path) -> bool {
    let value = path
        .to_string_lossy()
        .replace('/', "\\")
        .trim_start_matches("\\\\?\\")
        .to_lowercase();
    value.split('\\').any(|part| {
        matches!(
            part,
            "iclouddrive"
                | "icloud photos archive"
                | "onedrive"
                | "dropbox"
                | "google drive"
                | "proton drive"
        ) || part.starts_with("onedrive - ")
    }) || ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"]
        .iter()
        .any(|key| {
            std::env::var_os(key).is_some_and(|root| {
                let root = root.to_string_lossy().replace('/', "\\").to_lowercase();
                !root.is_empty() && (value == root || value.starts_with(&format!("{root}\\")))
            })
        })
}

fn relative(value: &str) -> Result<PathBuf> {
    ensure!(
        !value.is_empty() && !value.contains('\\') && !value.contains(':') && !value.contains('\0'),
        "Unsafe relative path {value:?}"
    );
    let path = PathBuf::from(value);
    ensure!(
        path.components()
            .all(|part| matches!(part, Component::Normal(_))),
        "Unsafe relative path {value:?}"
    );
    for name in value.split('/') {
        ensure!(
            !name.ends_with(['.', ' ']),
            "Ambiguous Windows path {value:?}"
        );
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit()),
            "Reserved Windows path {value:?}"
        );
    }
    Ok(path)
}

fn excluded(value: &str) -> bool {
    value.split('/').any(|part| {
        let part = part.to_ascii_lowercase();
        matches!(
            part.as_str(),
            ".git"
                | ".agents"
                | ".claude"
                | ".codex"
                | ".grok"
                | ".gemini"
                | ".muse"
                | ".cursor"
                | ".windsurf"
                | ".opencode"
                | ".aider"
                | ".goose"
                | ".cline"
                | ".openhands"
                | ".pi"
                | ".droid"
                | ".vscode"
                | ".github-copilot"
                | "agents.md"
                | "agents.override.md"
                | "agent.md"
                | "claude.md"
                | "claude.local.md"
                | "gemini.md"
                | "grok.md"
                | "memory.md"
                | "copilot-instructions.md"
                | ".cursorrules"
                | ".windsurfrules"
                | ".mcp.json"
                | "mcp.json"
                | "hooks.json"
                | ".gitconfig"
                | ".npmrc"
                | ".pypirc"
                | ".netrc"
                | ".env"
        ) || part.starts_with(".env.")
            || part.starts_with(".aider.")
    })
}

fn leases(path: &Path, create: bool) -> Result<Vec<DirectoryLease>> {
    ensure!(
        path.is_absolute() && !cloud(path),
        "Workspace path must be absolute and outside cloud roots: {}",
        path.display()
    );
    let mut current = PathBuf::new();
    let mut held = Vec::new();
    for component in path.components() {
        ensure!(
            !matches!(component, Component::ParentDir | Component::CurDir),
            "Unnormalized path {}",
            path.display()
        );
        current.push(component.as_os_str());
        if !current.has_root() {
            continue;
        }
        if create && !current.try_exists()? {
            fs::create_dir(&current).with_context(|| format!("creating {}", current.display()))?;
        }
        held.push(file_tx::lease_directory(&current)?);
    }
    Ok(held)
}

fn existing_parents(path: &Path) -> Result<Vec<DirectoryLease>> {
    ensure!(
        path.is_absolute() && !cloud(path),
        "Unsafe directory {}",
        path.display()
    );
    let mut current = PathBuf::new();
    let mut held = Vec::new();
    for component in path.components() {
        ensure!(
            !matches!(component, Component::ParentDir | Component::CurDir),
            "Unnormalized directory"
        );
        current.push(component.as_os_str());
        if !current.has_root() {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error).context("resolving existing directory ancestors"),
            Ok(_) => held.push(file_tx::lease_directory(&current)?),
        }
    }
    Ok(held)
}

fn pinned(path: &Path) -> Result<Option<File>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                metadata.is_file() && metadata.file_attributes() & 0x400 == 0,
                "Not a regular non-reparse file: {}",
                path.display()
            );
            Ok(Some(file_tx::open_pinned(path)?))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("opening {}", path.display())),
    }
}

fn read_file(path: &Path) -> Result<Option<Vec<u8>>> {
    let Some(mut file) = pinned(path)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn git_command(snapshot: &Snapshot, cwd: &Path) -> Command {
    let mut command = Command::new(&snapshot.git);
    command.env_clear().current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    for name in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("HOME", &snapshot.data)
        .env("USERPROFILE", &snapshot.data)
        .env("XDG_CONFIG_HOME", snapshot.data.join("git-home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", snapshot.data.join("git-global.config"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.untrackedCache=false",
            "-c",
            "core.protectNTFS=true",
        ])
        .arg("-c")
        .arg(format!(
            "core.hooksPath={}",
            snapshot.data.join("git-template").display()
        ))
        .arg("-c")
        .arg(format!(
            "core.excludesFile={}",
            snapshot.data.join("git-global.config").display()
        ));
    command
}

fn inventory(snapshot: &Snapshot, root: &Path) -> Result<BTreeSet<String>> {
    let output = git_command(snapshot, root)
        .arg("-c")
        .arg(format!("core.worktree={}", root.display()))
        .args([
            "-c",
            "core.bare=false",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()?;
    ensure!(
        output.status.success(),
        "Git inventory failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|item| !item.is_empty())
        .map(|item| {
            let value = std::str::from_utf8(item)
                .context("Non-UTF8 filenames are unsupported")?
                .to_owned();
            relative(&value)?;
            Ok(value)
        })
        .collect()
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Missing destination parent")?;
    let _held = leases(parent, true)?;
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn publish_new(path: &Path, bytes: &[u8]) -> Result<file_tx::FileIdentity> {
    let parent = path.parent().context("Missing publication parent")?;
    let _held = leases(parent, false)?;
    let parent = file_tx::lease_directory(parent)?;
    let mut stage = OwnedStagingFile::create_beside(path)?;
    stage.file()?.write_all(bytes)?;
    stage.file()?.sync_all()?;
    rename(
        stage.file()?,
        &parent,
        path.file_name().context("Missing publication name")?,
    )?;
    let identity = stage.identity();
    stage.mark_kept();
    Ok(identity)
}

pub fn import(root: &Path, generation: &Path, git: &Path) -> Result<Snapshot> {
    let root_leases = leases(root, false)?;
    let generation_leases = leases(generation, false)?;
    let root = root_leases
        .last()
        .context("Missing project root")?
        .resolved_path()
        .to_path_buf();
    let generation = generation_leases
        .last()
        .context("Missing generation")?
        .resolved_path()
        .to_path_buf();
    ensure!(
        !root.starts_with(&generation) && !generation.starts_with(&root),
        "Project and generation must be independent directories"
    );
    if generation.join("snapshot-data").try_exists()?
        || generation.join("snapshot.json").try_exists()?
    {
        let snapshot = load(&generation)?;
        ensure!(
            snapshot.root == root,
            "This generation already belongs to a different source project"
        );
        return Ok(snapshot);
    }
    ensure!(
        git.is_absolute() && !cloud(git) && git.is_file(),
        "An existing absolute Git binary is required"
    );
    let _git_image = file_tx::open_launch_image(git)?;
    let _git_metadata = leases(&root.join(".git"), false).context("Use a repository root with ordinary .git metadata; linked worktrees and submodules require separate import")?;
    let staging = generation.join(format!(
        "snapshot-import-{}",
        time::OffsetDateTime::now_utc().unix_timestamp_nanos()
    ));
    fs::create_dir(&staging)?;
    let outcome = (|| -> Result<Snapshot> {
        let mut snapshot = Snapshot {
            id: format!(
                "workspace-{}",
                &digest(root.to_string_lossy().as_bytes())[..24]
            ),
            root: root.clone(),
            shadow: staging.join("workspace"),
            generation: generation.clone(),
            data: staging.clone(),
            git: git.to_path_buf(),
            preimages: BTreeMap::new(),
            excluded: vec![".git".into()],
            git_config_hash: String::new(),
        };
        fs::create_dir(&snapshot.shadow)?;
        fs::create_dir(snapshot.data.join("preimages"))?;
        fs::create_dir(snapshot.data.join("git-template"))?;
        write_new(&snapshot.data.join("git-global.config"), b"")?;
        write_new(&snapshot.data.join("workspace.lock"), b"")?;
        for value in inventory(&snapshot, &snapshot.root)? {
            if excluded(&value) {
                snapshot.excluded.push(value);
                continue;
            }
            let path = relative(&value)?;
            let source = snapshot.root.join(&path);
            let _parents = existing_parents(source.parent().context("Missing source parent")?)?;
            let bytes = read_file(&source)?;
            snapshot
                .preimages
                .insert(value, bytes.as_deref().map(digest));
            if let Some(bytes) = bytes {
                write_new(&snapshot.shadow.join(&path), &bytes)?;
                write_new(&snapshot.data.join("preimages").join(&path), &bytes)?;
            }
        }
        let output = git_command(&snapshot, &snapshot.shadow)
            .arg("init")
            .arg(format!(
                "--template={}",
                snapshot.data.join("git-template").display()
            ))
            .output()?;
        ensure!(
            output.status.success(),
            "Clean Git initialization failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        snapshot.git_config_hash = digest(
            &read_file(&snapshot.shadow.join(".git/config"))?
                .context("Missing clean Git config")?,
        );
        snapshot.data = generation.join("snapshot-data");
        snapshot.shadow = snapshot.data.join("workspace");
        write_new(
            &staging.join("snapshot.json"),
            &serde_json::to_vec_pretty(&snapshot)?,
        )?;
        fs::rename(&staging, &snapshot.data)
            .context("publishing the independent snapshot bundle")?;
        publish_new(
            &generation.join("snapshot.json"),
            &serde_json::to_vec_pretty(&snapshot)?,
        )?;
        Ok(snapshot)
    })();
    outcome.with_context(|| format!("Snapshot import failed; any incomplete owned staging is retained at {}. Retry uses a fresh staging bundle", staging.display()))
}

pub fn load(generation: &Path) -> Result<Snapshot> {
    let held = leases(generation, false)?;
    let generation = held.last().context("Missing generation")?.resolved_path();
    ensure!(
        !generation
            .join("snapshot-apply-pending.json")
            .try_exists()?,
        "An apply transaction needs recovery; inspect {} before further work",
        generation.join("snapshot-apply-pending.json").display()
    );
    let primary = read_file(&generation.join("snapshot.json"))?;
    let bytes = if let Some(bytes) = &primary {
        bytes.clone()
    } else {
        let _bundle = leases(&generation.join("snapshot-data"), false)?;
        read_file(&generation.join("snapshot-data/snapshot.json"))?
            .context("No snapshot has been imported")?
    };
    let snapshot: Snapshot = serde_json::from_slice(&bytes)?;
    ensure!(
        snapshot.generation == generation
            && snapshot.data == generation.join("snapshot-data")
            && snapshot.shadow == snapshot.data.join("workspace"),
        "Snapshot paths do not match this generation"
    );
    leases(&snapshot.root, false)?;
    leases(&snapshot.shadow, false)?;
    ensure!(
        snapshot
            .preimages
            .keys()
            .all(|path| relative(path).is_ok() && !excluded(path)),
        "Snapshot contains unauthorized paths"
    );
    if primary.is_none() {
        publish_new(&generation.join("snapshot.json"), &bytes)?;
    }
    Ok(snapshot)
}

fn lease_use(snapshot: &Snapshot, exclusive: bool) -> Result<File> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    let _parents = leases(&snapshot.data, false)?;
    let file = OpenOptions::new().read(true).write(exclusive)
        .share_mode(if exclusive { 0 } else { windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ })
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(snapshot.data.join("workspace.lock"))
        .context("Workspace is in use; stop active workers before applying, or wait for reconciliation before starting a worker")?;
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.file_attributes() & 0x400 == 0,
        "Invalid workspace ownership lock"
    );
    Ok(file)
}

pub fn ensure_clean(snapshot: &Snapshot) -> Result<()> {
    ensure!(
        !snapshot
            .generation
            .join("snapshot-apply-pending.json")
            .try_exists()?,
        "Pending apply requires recovery before worker launch"
    );
    let _held = leases(&snapshot.shadow, false)?;
    let mut directories = BTreeSet::from([PathBuf::new()]);
    let mut names = inventory(snapshot, &snapshot.shadow)?;
    names.extend(snapshot.preimages.keys().cloned());
    let mut blocked = Vec::new();
    for name in names {
        if excluded(&name) {
            blocked.push(name);
            continue;
        }
        let path = relative(&name)?;
        let mut parent = path.parent();
        while let Some(path) = parent {
            directories.insert(path.to_path_buf());
            parent = path.parent();
        }
    }
    for relative in directories {
        let path = snapshot.shadow.join(relative);
        let _parents = existing_parents(&path)?;
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("reading relevant workspace directories"),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let name = path
                .strip_prefix(&snapshot.shadow)?
                .to_str()
                .context("Non-UTF8 filename")?
                .replace('\\', "/");
            if name == ".git" {
                continue;
            }
            if excluded(&name) {
                blocked.push(name);
                continue;
            }
            use std::os::windows::fs::MetadataExt;
            ensure!(
                fs::symlink_metadata(&path)?.file_attributes() & 0x400 == 0,
                "Workspace reparse point is unsupported: {name}"
            );
        }
    }
    ensure!(
        blocked.is_empty(),
        "Automatic-load paths appeared; worker launch refused: {}",
        blocked.join(", ")
    );
    let _git = leases(&snapshot.shadow.join(".git"), false)?;
    ensure!(
        read_file(&snapshot.shadow.join(".git/config"))?
            .as_deref()
            .map(digest)
            .as_deref()
            == Some(snapshot.git_config_hash.as_str()),
        "Isolated Git config changed; worker launch refused"
    );
    ensure!(
        !snapshot.shadow.join(".git/hooks").try_exists()?,
        "Isolated Git hooks appeared; worker launch refused"
    );
    Ok(())
}

fn changes(snapshot: &Snapshot) -> Result<Vec<Change>> {
    ensure_clean(snapshot)?;
    let mut names = inventory(snapshot, &snapshot.shadow)?;
    names.extend(snapshot.preimages.keys().cloned());
    let mut changes = Vec::new();
    for value in names {
        if excluded(&value) {
            continue;
        }
        let path = snapshot.shadow.join(relative(&value)?);
        let _held = existing_parents(path.parent().context("Missing workspace parent")?)?;
        let after_hash = read_file(&path)?.as_deref().map(digest);
        let before_hash = snapshot.preimages.get(&value).cloned().flatten();
        if before_hash != after_hash {
            let kind = if before_hash.is_none() {
                "added"
            } else if after_hash.is_none() {
                "deleted"
            } else {
                "modified"
            };
            changes.push(Change {
                path: value,
                kind: kind.into(),
                before_hash,
                after_hash,
            });
        }
    }
    Ok(changes)
}

pub fn diff(snapshot: &Snapshot) -> Result<SnapshotDiff> {
    let changes = changes(snapshot)?;
    let report = snapshot.generation.join("snapshot-diff.json");
    let patch = snapshot.generation.join("snapshot-diff.patch");
    let result = SnapshotDiff {
        workspace_id: snapshot.id.clone(),
        root: snapshot.root.clone(),
        shadow: snapshot.shadow.clone(),
        changes,
        report: report.clone(),
        patch: patch.clone(),
    };
    let mut content = Vec::new();
    for change in &result.changes {
        let before = snapshot
            .data
            .join("preimages")
            .join(relative(&change.path)?);
        let after = snapshot.shadow.join(relative(&change.path)?);
        let output = git_command(snapshot, &snapshot.generation)
            .args([
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--binary",
                "--",
            ])
            .arg(if change.before_hash.is_some() {
                before.as_os_str()
            } else {
                OsStr::new("/dev/null")
            })
            .arg(if change.after_hash.is_some() {
                after.as_os_str()
            } else {
                OsStr::new("/dev/null")
            })
            .output()?;
        ensure!(
            output
                .status
                .code()
                .is_some_and(|code| code == 0 || code == 1),
            "Diff generation failed for {}: {}",
            change.path,
            String::from_utf8_lossy(&output.stderr)
        );
        content.extend(output.stdout);
    }
    file_tx::replace_file_contents(&patch, &content)?;
    file_tx::replace_file_contents(&report, &serde_json::to_vec_pretty(&result)?)?;
    Ok(result)
}

fn preflight(
    snapshot: &Snapshot,
    changes: &[Change],
) -> Result<Vec<(Option<File>, Vec<DirectoryLease>)>> {
    let mut originals = Vec::new();
    let mut conflicts = Vec::new();
    for change in changes {
        let path = snapshot.root.join(relative(&change.path)?);
        let held = existing_parents(path.parent().context("Missing destination parent")?)?;
        let mut original = pinned(&path)?;
        let mut bytes = Vec::new();
        if let Some(file) = &mut original {
            file.read_to_end(&mut bytes)?;
        }
        let observed = original.as_ref().map(|_| digest(&bytes));
        if observed != change.before_hash {
            conflicts.push(change.path.clone());
        }
        originals.push((original, held));
    }
    ensure!(
        conflicts.is_empty(),
        "Original files changed; nothing applied: {}",
        conflicts.join(", ")
    );
    Ok(originals)
}

fn confirm(snapshot: &Snapshot, diff: &SnapshotDiff) -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn MessageBoxW(
            window: *mut std::ffi::c_void,
            text: *const u16,
            caption: *const u16,
            flags: u32,
        ) -> i32;
    }
    let mut text = format!(
        "Apply {} file changes to the ORIGINAL project?\n\n{}\n\n",
        diff.changes.len(),
        snapshot.root.display()
    );
    for change in diff.changes.iter().take(20) {
        text.push_str(&format!("{}: {}\n", change.kind, change.path));
    }
    text.push_str(&format!("\nComplete path/hash report: {}\nContent diff: {}\n\nCtrl+C copies this dialog. Unrelated original changes remain untouched.", diff.report.display(), diff.patch.display()));
    let text = text.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let caption = "Apply isolated workspace changes?"
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            caption.as_ptr(),
            0x4 | 0x30 | 0x100 | 0x10000 | 0x40000,
        ) == 6
    }
}

fn rename(file: &File, directory: &DirectoryLease, leaf: &OsStr) -> Result<()> {
    use std::mem::{offset_of, size_of};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Wdk::Storage::FileSystem::{
        FILE_RENAME_INFORMATION, FileRenameInformation, NtSetInformationFile,
    };
    use windows_sys::Win32::Foundation::RtlNtStatusToDosError;
    use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
    let name = leaf.encode_wide().collect::<Vec<_>>();
    let bytes = name.len() * 2;
    let length = size_of::<FILE_RENAME_INFORMATION>()
        .max(offset_of!(FILE_RENAME_INFORMATION, FileName) + bytes);
    let mut storage = vec![0_u64; length.div_ceil(8)];
    let information = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    unsafe {
        (*information).Anonymous.ReplaceIfExists = false;
        (*information).RootDirectory = directory.file().as_raw_handle();
        (*information).FileNameLength = u32::try_from(bytes)?;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            (&raw mut (*information).FileName).cast::<u16>(),
            name.len(),
        );
        let mut status = IO_STATUS_BLOCK::default();
        let result = NtSetInformationFile(
            file.as_raw_handle(),
            &raw mut status,
            storage.as_ptr().cast(),
            u32::try_from(length)?,
            FileRenameInformation,
        );
        if result < 0 {
            return Err(std::io::Error::from_raw_os_error(
                RtlNtStatusToDosError(result) as i32,
            ))
            .context("publishing a snapshot file without replacement");
        }
    }
    Ok(())
}

struct Pending {
    destination: PathBuf,
    original: Option<File>,
    backup: Option<PathBuf>,
    backup_path: Option<PathBuf>,
    payload: Option<Vec<u8>>,
    _source: Option<File>,
    stage: Option<OwnedStagingFile>,
    parent: DirectoryLease,
    published: bool,
}

pub fn apply(snapshot: &Snapshot) -> Result<ApplyReport> {
    let _exclusive = lease_use(snapshot, true)?;
    let approved = diff(snapshot)?;
    drop(preflight(snapshot, &approved.changes)?);
    if approved.changes.is_empty() || !confirm(snapshot, &approved) {
        return Ok(ApplyReport {
            applied: false,
            files: Vec::new(),
            message: "No original files changed".into(),
            report: approved.report,
        });
    }
    ensure!(
        changes(snapshot)? == approved.changes,
        "Workspace changed after approval; review a fresh diff before applying"
    );
    let originals = preflight(snapshot, &approved.changes)?;
    let mut pending = Vec::new();
    let mut ancestor_leases = Vec::new();
    for (change, (original, held)) in approved.changes.iter().zip(originals) {
        ancestor_leases.extend(held);
        let relative = relative(&change.path)?;
        let destination = snapshot.root.join(&relative);
        let parent = destination.parent().context("Missing destination parent")?;
        ancestor_leases.extend(leases(parent, true)?);
        let parent = file_tx::lease_directory(parent)?;
        let mut payload = None;
        let mut source_pin = None;
        let stage = if let Some(expected) = &change.after_hash {
            let source = snapshot.shadow.join(&relative);
            let _held = leases(source.parent().context("Missing workspace parent")?, false)?;
            let mut file = pinned(&source)?.context("Approved workspace file disappeared")?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            source_pin = Some(file);
            ensure!(
                digest(&bytes) == *expected,
                "Workspace file changed after approval: {}",
                change.path
            );
            let mut stage = OwnedStagingFile::create_beside(&destination)?;
            stage.file()?.write_all(&bytes)?;
            stage.file()?.sync_all()?;
            payload = Some(bytes);
            Some(stage)
        } else {
            None
        };
        let backup_path = original.as_ref().map(|_| {
            parent
                .resolved_path()
                .join(file_tx::backup_leaf_name(&destination))
        });
        pending.push(Pending {
            destination,
            original,
            backup: None,
            backup_path,
            payload,
            _source: source_pin,
            stage,
            parent,
            published: false,
        });
    }
    let mut next_snapshot = snapshot.clone();
    for change in &approved.changes {
        next_snapshot
            .preimages
            .insert(change.path.clone(), change.after_hash.clone());
    }
    let mut recovery_files = Vec::new();
    for (change, item) in approved.changes.iter().zip(&pending) {
        let original_identity = item
            .original
            .as_ref()
            .map(file_tx::identity_of)
            .transpose()?;
        recovery_files.push(serde_json::json!({
            "path":change.path,"before_hash":change.before_hash,"after_hash":change.after_hash,
            "backup":item.backup_path,
            "original_identity":original_identity.map(|identity|format!("{}:{}",identity.volume_serial(),identity.file_id_hex())),
            "stage":item.stage.as_ref().map(|stage|stage.path()),
            "stage_identity":item.stage.as_ref().map(|stage|format!("{}:{}",stage.identity().volume_serial(),stage.identity().file_id_hex()))
        }));
    }
    let journal = snapshot.generation.join("snapshot-apply-pending.json");
    let journal_identity = publish_new(
        &journal,
        &serde_json::to_vec_pretty(&serde_json::json!({
            "workspace_id":snapshot.id,"root":snapshot.root,"prior_snapshot":snapshot,"next_snapshot":next_snapshot,"files":recovery_files
        }))?,
    )?;
    let outcome = (|| -> Result<()> {
        for item in &mut pending {
            if let Some(original) = &item.original {
                let backup = item
                    .backup_path
                    .as_ref()
                    .context("Missing planned backup")?;
                rename(
                    original,
                    &item.parent,
                    backup.file_name().context("Missing backup name")?,
                )?;
                item.backup = Some(backup.clone());
            }
            if let Some(stage) = &mut item.stage {
                rename(
                    stage.file()?,
                    &item.parent,
                    item.destination
                        .file_name()
                        .context("Missing destination name")?,
                )?;
                item.published = true;
            }
        }
        Ok(())
    })();
    if let Err(error) = outcome {
        let mut rollback_failures = Vec::new();
        for item in pending.iter_mut().rev() {
            if item.published
                && let Some(stage) = &mut item.stage
            {
                let leaf = file_tx::stage_leaf_name(&item.destination);
                if let Err(error) = rename(stage.file()?, &item.parent, &leaf) {
                    stage.mark_kept();
                    rollback_failures.push(format!(
                        "{}: {error:#}; backup {:?}",
                        item.destination.display(),
                        item.backup
                    ));
                    continue;
                }
            }
            if item.backup.is_some()
                && let Some(original) = &item.original
                && let Err(error) = rename(
                    original,
                    &item.parent,
                    item.destination
                        .file_name()
                        .context("Missing original name")?,
                )
            {
                rollback_failures.push(error.to_string());
            }
        }
        if rollback_failures.is_empty() {
            ensure!(
                file_tx::remove_recorded_object(&journal, journal_identity)?,
                "Rollback completed but recovery journal changed: {}",
                journal.display()
            );
            bail!("Apply failed and file changes were rolled back: {error:#}");
        }
        bail!(
            "Apply failed: {error:#}. Rollback incomplete; retain backup files and inspect original project: {}",
            rollback_failures.join("; ")
        );
    }
    for item in &mut pending {
        if let Some(stage) = &mut item.stage {
            stage.mark_kept();
        }
    }
    let baseline_result = (|| -> Result<()> {
        for (change, item) in approved.changes.iter().zip(&pending) {
            if let Some(bytes) = &item.payload {
                let path = snapshot
                    .data
                    .join("preimages")
                    .join(relative(&change.path)?);
                let _held = leases(path.parent().context("Missing preimage parent")?, true)?;
                file_tx::replace_file_contents(&path, bytes)?;
            }
        }
        let bytes = serde_json::to_vec_pretty(&next_snapshot)?;
        file_tx::replace_file_contents(&snapshot.data.join("snapshot.json"), &bytes)?;
        file_tx::replace_file_contents(&snapshot.generation.join("snapshot.json"), &bytes)?;
        Ok(())
    })();
    if let Err(error) = baseline_result {
        bail!(
            "Original changes were applied, but baseline persistence failed: {error:#}. Recovery is required using {}; do not repeat apply",
            journal.display()
        );
    }
    let mut cleanup = Vec::new();
    for item in pending {
        if let (Some(original), Some(backup)) = (item.original, item.backup) {
            let identity = file_tx::identity_of(&original)?;
            drop(original);
            match file_tx::remove_recorded_object(&backup, identity) {
                Ok(true) => {}
                _ => cleanup.push(backup.display().to_string()),
            }
        }
    }
    ensure!(
        file_tx::remove_recorded_object(&journal, journal_identity)?,
        "Changes and baseline saved, but recovery journal changed: {}",
        journal.display()
    );
    Ok(ApplyReport {
        applied: true,
        files: approved
            .changes
            .into_iter()
            .map(|change| change.path)
            .collect(),
        message: if cleanup.is_empty() {
            "Approved changes applied".into()
        } else {
            format!(
                "Changes applied; retained backups require cleanup: {}",
                cleanup.join(", ")
            )
        },
        report: approved.report,
    })
}
