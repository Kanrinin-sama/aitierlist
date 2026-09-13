use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::agent_setup::ImmutableArtifact;
use crate::research_packet::EvidencePacket;
use crate::settings::Settings;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
pub struct WorkspacePaths {
    pub root: PathBuf,
    pub projects: PathBuf,
    pub briefs: PathBuf,
    pub decisions: PathBuf,
    pub research: PathBuf,
    pub deliverables: PathBuf,
    pub scratch: PathBuf,
    pub guide: PathBuf,
    pub index: PathBuf,
    pub readme: PathBuf,
    pub docs_index: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct WorkspaceIndex {
    version: u8,
    projects: String,
    briefs: String,
    decisions: String,
    research: String,
    deliverables: String,
    scratch: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct ResearchIndex {
    version: u8,
    packets: Vec<ResearchEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case")]
struct ResearchEntry {
    packet_id: String,
    task_id: String,
    packet_path: String,
    findings_path: String,
    digest: String,
    consuming_decision: String,
}

pub fn default_root() -> Result<PathBuf> {
    let home = directories::UserDirs::new().context("User home directory is unavailable")?;
    Ok(home.home_dir().join("aitierlist-workspace"))
}

pub fn resolve(settings: &Settings) -> Result<WorkspacePaths> {
    let root = settings
        .collaboration_root
        .clone()
        .map_or_else(default_root, Ok)?;
    ensure!(
        root.is_absolute(),
        "Collaboration workspace root must be absolute"
    );
    Ok(paths(root))
}

pub fn resolve_root(root: &Path) -> Result<WorkspacePaths> {
    ensure!(
        root.is_absolute(),
        "Collaboration workspace root must be absolute"
    );
    validate_root(root)?;
    let paths = paths(root.to_path_buf());
    reject_chain(&paths.research, root)?;
    Ok(paths)
}

#[cfg(windows)]
pub fn pick_root() -> Result<Option<PathBuf>> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    };
    use windows_sys::Win32::UI::Shell::{
        BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW,
        SHGetPathFromIDListW,
    };

    ensure!(
        unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) } >= 0,
        "Cannot initialize the native folder picker apartment"
    );
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }
    let _apartment = Apartment;
    let title: Vec<u16> = "Choose the aitierlist collaboration workspace\0"
        .encode_utf16()
        .collect();
    let mut display = [0_u16; 260];
    let dialog = BROWSEINFOW {
        hwndOwner: std::ptr::null_mut(),
        pidlRoot: std::ptr::null_mut(),
        pszDisplayName: display.as_mut_ptr(),
        lpszTitle: title.as_ptr(),
        ulFlags: BIF_NEWDIALOGSTYLE | BIF_RETURNONLYFSDIRS,
        lpfn: None,
        lParam: 0,
        iImage: 0,
    };
    let item = unsafe { SHBrowseForFolderW(&dialog) };
    if item.is_null() {
        return Ok(None);
    }
    let mut path = [0_u16; 32_768];
    let success = unsafe { SHGetPathFromIDListW(item, path.as_mut_ptr()) } != 0;
    unsafe { CoTaskMemFree(item.cast()) };
    ensure!(success, "Selected folder has no filesystem path");
    let end = path
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(path.len());
    Ok(Some(PathBuf::from(std::ffi::OsString::from_wide(
        &path[..end],
    ))))
}

#[cfg(not(windows))]
pub fn pick_root() -> Result<Option<PathBuf>> {
    Ok(None)
}

pub fn prepare(root: &Path) -> Result<WorkspacePaths> {
    ensure!(
        root.is_absolute(),
        "Collaboration workspace root must be absolute"
    );
    let _lock = workspace_lock()?;
    validate_root(root)?;
    std::fs::create_dir_all(root).with_context(|| format!("creating {}", root.display()))?;
    let paths = paths(root.to_path_buf());
    for directory in [
        &paths.projects,
        &paths.briefs,
        &paths.decisions,
        &paths.research,
        &paths.deliverables,
        &paths.scratch,
    ] {
        reject_chain(directory, root)?;
        std::fs::create_dir_all(directory)
            .with_context(|| format!("creating {}", directory.display()))?;
    }
    for file in [
        &paths.guide,
        &paths.readme,
        &paths.docs_index,
        &paths.index,
        &root.join(".gitignore"),
        &paths.research.join("index.json"),
        &paths.research.join("index.md"),
    ] {
        reject_chain(file, root)?;
    }
    write_new(&paths.guide, GUIDE.as_bytes())?;
    write_new(&paths.readme, README.as_bytes())?;
    write_new(&paths.docs_index, DOCS_INDEX.as_bytes())?;
    let index = WorkspaceIndex {
        version: 1,
        projects: relative(&paths.projects, root)?,
        briefs: relative(&paths.briefs, root)?,
        decisions: relative(&paths.decisions, root)?,
        research: relative(&paths.research, root)?,
        deliverables: relative(&paths.deliverables, root)?,
        scratch: relative(&paths.scratch, root)?,
    };
    write_immutable(&paths.index, &serde_json::to_vec_pretty(&index)?)?;
    ensure_gitignore(&root.join(".gitignore"))?;
    write_new(
        &paths.research.join("index.json"),
        &serde_json::to_vec_pretty(&ResearchIndex {
            version: 1,
            packets: Vec::new(),
        })?,
    )?;
    let research_index: ResearchIndex =
        serde_json::from_slice(&std::fs::read(paths.research.join("index.json"))?)?;
    write_new(
        &paths.research.join("index.md"),
        render_research_index(&research_index).as_bytes(),
    )?;
    if !root.join(".git").exists() {
        let mut command = std::process::Command::new(crate::host_install::git_binary()?);
        command.args(["init", "--quiet"]).current_dir(root);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let status = command
            .status()
            .context("starting git init for the collaboration workspace")?;
        ensure!(
            status.success(),
            "git init failed for the collaboration workspace"
        );
    }
    Ok(paths)
}

pub fn publish_research(
    paths: &WorkspacePaths,
    task_id: &str,
    packet: &EvidencePacket,
    accepted: &ImmutableArtifact,
    findings: &str,
) -> Result<ImmutableArtifact> {
    let _lock = workspace_lock()?;
    validate_root(&paths.root)?;
    ensure!(
        stable_id(task_id) && stable_id(&packet.id) && packet.task_id == task_id,
        "Research task or packet identity is invalid"
    );
    ensure!(
        accepted.id == packet.id
            && accepted.producer_attempt_id == packet.producer_attempt_id
            && accepted.parent_artifact_id.is_some()
            && accepted.accepted_at.is_some(),
        "Accepted research artifact and packet identity or lineage differ"
    );
    ensure!(!findings.trim().is_empty(), "Research findings are empty");
    let directory = paths.research.join(task_id);
    reject_chain(&directory, &paths.root)?;
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("creating {}", directory.display()))?;
    let packet_path = directory.join(format!("{}.json", packet.id));
    let findings_path = directory.join(format!("{}.md", packet.id));
    reject_chain(&packet_path, &paths.root)?;
    reject_chain(&findings_path, &paths.root)?;
    let packet_bytes = serde_json::to_vec(packet)?;
    let digest = accepted.digest.clone();
    ensure!(
        hex::encode(Sha256::digest(&packet_bytes)) == digest,
        "Accepted research digest differs from packet content"
    );
    write_immutable(&packet_path, &packet_bytes)?;
    write_immutable(&findings_path, findings.as_bytes())?;
    let index_path = paths.research.join("index.json");
    reject_chain(&index_path, &paths.root)?;
    reject_chain(&paths.research.join("index.md"), &paths.root)?;
    let mut index: ResearchIndex = serde_json::from_slice(
        &std::fs::read(&index_path).with_context(|| format!("reading {}", index_path.display()))?,
    )?;
    let entry = ResearchEntry {
        packet_id: packet.id.clone(),
        task_id: task_id.to_owned(),
        packet_path: relative(&packet_path, &paths.root)?,
        findings_path: relative(&findings_path, &paths.root)?,
        digest: digest.clone(),
        consuming_decision: packet.consuming_decision.clone(),
    };
    if let Some(existing) = index
        .packets
        .iter()
        .find(|existing| existing.packet_id == packet.id)
    {
        ensure!(
            serde_json::to_value(existing)? == serde_json::to_value(&entry)?,
            "Research packet ID is already published with different content"
        );
        crate::file_tx::replace_file_contents(
            &paths.research.join("index.md"),
            render_research_index(&index).as_bytes(),
        )?;
        return Ok(published_artifact(accepted, &packet_path));
    }
    index.packets.push(entry);
    index
        .packets
        .sort_by(|left, right| left.packet_id.cmp(&right.packet_id));
    crate::file_tx::replace_file_contents(&index_path, &serde_json::to_vec_pretty(&index)?)?;
    crate::file_tx::replace_file_contents(
        &paths.research.join("index.md"),
        render_research_index(&index).as_bytes(),
    )?;
    Ok(published_artifact(accepted, &packet_path))
}

fn published_artifact(accepted: &ImmutableArtifact, packet_path: &Path) -> ImmutableArtifact {
    let mut published = accepted.clone();
    published.path = packet_path.to_string_lossy().into_owned();
    published
}

fn paths(root: PathBuf) -> WorkspacePaths {
    WorkspacePaths {
        projects: root.join("projects"),
        briefs: root.join("docs/briefs"),
        decisions: root.join("docs/decisions"),
        research: root.join("research"),
        deliverables: root.join("deliverables"),
        scratch: root.join("scratch"),
        guide: root.join("WORKSPACE.md"),
        index: root.join("workspace-index.v1.json"),
        readme: root.join("README.md"),
        docs_index: root.join("docs/index.md"),
        root,
    }
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("creating {}", path.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", path.display()))?;
    Ok(())
}

struct WorkspaceLock {
    _file: std::fs::File,
}

fn workspace_lock() -> Result<WorkspaceLock> {
    let directory = crate::agent_setup::paths()?.directory;
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("workspace-publish.lock");
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
            .context("The workspace research index is busy; retry publication")?;
        Ok(WorkspaceLock { _file: file })
    }
    #[cfg(not(windows))]
    {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        Ok(WorkspaceLock { _file: file })
    }
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Result<()> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(bytes)
                .with_context(|| format!("writing {}", path.display()))?;
            file.sync_all()
                .with_context(|| format!("syncing {}", path.display()))?;
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure!(
                std::fs::read(path).with_context(|| format!("reading {}", path.display()))?
                    == bytes,
                "Immutable workspace artifact already exists with different content"
            );
            Ok(())
        }
        Err(error) => Err(error).with_context(|| format!("creating {}", path.display())),
    }
}

fn ensure_gitignore(path: &Path) -> Result<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    if existing.lines().any(|line| line.trim() == "/scratch/") {
        return Ok(());
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str("/scratch/\n");
    crate::file_tx::replace_file_contents(path, updated.as_bytes())
}

fn validate_root(root: &Path) -> Result<()> {
    ensure!(
        !root.components().any(|component| matches!(
            component,
            std::path::Component::CurDir | std::path::Component::ParentDir
        )),
        "Collaboration workspace path must be normalized"
    );
    let lexical = root
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    ensure!(
        ![
            "\\onedrive",
            "\\dropbox",
            "\\iclouddrive",
            "\\icloud photos"
        ]
        .iter()
        .any(|marker| lexical.contains(marker)),
        "Collaboration workspace must be on a local non-cloud path"
    );
    let mut cursor = PathBuf::new();
    let mut existing = None;
    for component in root.components() {
        cursor.push(component.as_os_str());
        match std::fs::symlink_metadata(&cursor) {
            Ok(_) => {
                reject_indirection(&cursor)?;
                existing = Some(cursor.clone());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", cursor.display()));
            }
        }
    }
    let existing = existing.context("Collaboration workspace has no existing ancestor")?;
    let resolved = existing
        .canonicalize()
        .with_context(|| format!("resolving {}", existing.display()))?;
    let value = resolved
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    ensure!(
        ![
            "\\onedrive",
            "\\dropbox",
            "\\iclouddrive",
            "\\icloud photos"
        ]
        .iter()
        .any(|marker| value.contains(marker)),
        "Collaboration workspace must be on a local non-cloud path"
    );
    Ok(())
}

fn reject_chain(path: &Path, root: &Path) -> Result<()> {
    ensure!(
        path.starts_with(root),
        "Managed workspace path escaped its root"
    );
    let mut chain = path
        .ancestors()
        .take_while(|current| current.starts_with(root))
        .collect::<Vec<_>>();
    ensure!(
        chain.last().copied() == Some(root),
        "Managed workspace path has no collaboration root ancestor"
    );
    chain.reverse();
    for current in chain {
        match std::fs::symlink_metadata(current) {
            Ok(_) => reject_indirection(current)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", current.display()));
            }
        }
    }
    Ok(())
}

fn reject_indirection(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("inspecting {}", path.display())),
    };
    ensure!(
        !metadata.file_type().is_symlink(),
        "Workspace path cannot contain a symbolic link"
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "Workspace path cannot contain a reparse point"
        );
    }
    Ok(())
}

fn render_research_index(index: &ResearchIndex) -> String {
    let mut output = String::from(
        "# Research index\n\n| Packet | Task | Decision | Evidence | Findings |\n|---|---|---|---|---|\n",
    );
    for entry in &index.packets {
        let packet_path = entry
            .packet_path
            .strip_prefix("research/")
            .unwrap_or(&entry.packet_path);
        let findings_path = entry
            .findings_path
            .strip_prefix("research/")
            .unwrap_or(&entry.findings_path);
        output.push_str(&format!(
            "| {} | {} | {} | [{}]({}) | [findings]({}) |\n",
            markdown_cell(&entry.packet_id),
            markdown_cell(&entry.task_id),
            markdown_cell(&entry.consuming_decision),
            markdown_cell(&entry.digest),
            packet_path.replace(' ', "%20"),
            findings_path.replace(' ', "%20")
        ));
    }
    output
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace(['\r', '\n'], " ")
}

fn relative(path: &Path, root: &Path) -> Result<String> {
    Ok(path
        .strip_prefix(root)
        .context("Workspace path escaped its root")?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn stable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

const GUIDE: &str = r#"# Collaboration workspace

`workspace-index.v1.json` defines the shared layout. Put code repositories under `projects/`, worker briefs under `docs/briefs/`, durable decisions under `docs/decisions/`, evidence packets and readable findings under `research/<task-id>/`, final user artifacts under `deliverables/`, and temporary logs or briefs under `scratch/`. Git ignores `scratch/`. This workspace contains user work only; aitierlist keeps profiles, authentication, ledgers, holds, and generated runtime files in its private application data directory.

The conductor owns user communication, planning, exact briefs, dispatch, reconciliation, and acceptance. Implementation belongs to an eligible implementation worker. Use minimal clear code. Source files contain no explanatory prose comments. A necessary language-native `[anchor-id]` marker has one matching heading in the repository side file. Do not create tests or validator scaffolding; accept observable artifacts and state transitions. Read recorded quantities instead of estimating them. Do not invent thresholds, log-only counters, fake work, or duplicate accepted work.

Use only exact verified bindings and the permissions, network scope, sources, roots, exclusions, billing, and native meters compiled into the current policy. Keep scope within the authorized task and local workspace boundaries. Never commit, push, publish, deploy, or mutate production automatically.
"#;

const README: &str = "# aitierlist collaboration workspace\n\nShared user work lives here. See `WORKSPACE.md` for operating boundaries and `docs/index.md` for durable documents.\n";

const DOCS_INDEX: &str = "# Documents\n\n- `briefs/`: task briefs\n- `decisions/`: accepted decisions\n- `../research/`: evidence packets and findings\n- `../deliverables/`: final artifacts\n";
