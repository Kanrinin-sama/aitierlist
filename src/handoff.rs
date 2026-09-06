use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::file_tx::{self, FileIdentity, OwnedStagingFile};
use sha2::Digest;

pub const HELPER_SELECTOR: &str = "--aitierlist-update-helper?v1";

const HELPER_FAILURE_EXIT: u32 = 0xA1E5_0DDE;

const READY_WAIT: Duration = Duration::from_secs(30);
const PARENT_WAIT: Duration = Duration::from_secs(10 * 60);
const LOCK_WAIT: Duration = Duration::from_secs(60);

const MAX_CONTROL_BYTES: u64 = 64 * 1024;

pub fn state_dir() -> Result<PathBuf> {
    let dir = crate::update::data_dir()?;
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

pub struct UpdateLock {
    _file: File,
}

fn canonical_token(canonical: &Path) -> String {
    let spelling = canonical.to_string_lossy().to_lowercase();
    hex::encode(sha2::Sha256::digest(spelling.as_bytes()))[..32].to_string()
}

pub fn lock_leaf_name(canonical: &Path) -> String {
    format!("update-{}.lock", canonical_token(canonical))
}

impl UpdateLock {
    pub fn acquire(canonical: &Path) -> Result<Self> {
        let path = state_dir()?.join(lock_leaf_name(canonical));
        Self::open(&path)
    }

    pub fn acquire_waiting(canonical: &Path, limit: Duration) -> Result<Self> {
        let deadline = Instant::now() + limit;
        loop {
            match Self::acquire(canonical) {
                Ok(lock) => return Ok(lock),
                Err(error) if Instant::now() >= deadline => return Err(error),
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    fn open(path: &Path) -> Result<Self> {
        use std::os::windows::fs::OpenOptionsExt;
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(path)
            .context(
                "another AITIERLIST process is already applying an update to this executable; wait for it to finish and try again",
            )?;
        Ok(UpdateLock { _file: file })
    }
}

#[derive(Debug, Clone)]
pub struct ExpectedImage {
    pub size: u64,
    pub sha256: String,
    pub version: String,
    pub volume: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ImageFacts {
    pub identity: FileIdentity,
}

fn stamp_prefix() -> Vec<u8> {
    let mut prefix = Vec::with_capacity(19);
    prefix.extend_from_slice(b"AITIERLIST");
    prefix.push(b'_');
    prefix.extend_from_slice(b"VERSION");
    prefix.push(b'=');
    prefix
}

#[derive(Debug, Default)]
pub struct StampScan {
    expected_prefix: Vec<u8>,
    carry: Vec<u8>,
    found: std::collections::BTreeSet<String>,
}

const MAX_STAMP_VALUE: usize = 32;

impl StampScan {
    pub fn new() -> Self {
        StampScan {
            expected_prefix: stamp_prefix(),
            carry: Vec::new(),
            found: std::collections::BTreeSet::new(),
        }
    }

    fn window(&self) -> usize {
        self.expected_prefix.len() + MAX_STAMP_VALUE + 1
    }

    pub fn feed(&mut self, chunk: &[u8]) {
        let window = self.window();
        let mut buffer = std::mem::take(&mut self.carry);
        buffer.extend_from_slice(chunk);
        let decidable = buffer.len().saturating_sub(window - 1);
        self.scan(&buffer, decidable);
        let keep = buffer.len().min(window - 1);
        self.carry = buffer[buffer.len() - keep..].to_vec();
    }

    fn scan(&mut self, buffer: &[u8], decidable: usize) {
        let first = self.expected_prefix[0];
        let mut at = 0;
        while at < decidable {
            let Some(offset) = buffer[at..decidable].iter().position(|byte| *byte == first) else {
                return;
            };
            let start = at + offset;
            self.consider(&buffer[start..]);
            at = start + 1;
        }
    }

    fn consider(&mut self, at: &[u8]) {
        if !at.starts_with(&self.expected_prefix) {
            return;
        }
        let rest = &at[self.expected_prefix.len()..];
        let Some(value) = stamp_value(rest) else {
            return;
        };
        if crate::update::parse_release_version(value).is_some() {
            self.found.insert(value.to_owned());
        }
    }

    pub fn finish(mut self) -> Vec<String> {
        let buffer = std::mem::take(&mut self.carry);
        let decidable = buffer
            .len()
            .saturating_sub(self.expected_prefix.len().saturating_sub(1));
        self.scan(&buffer, decidable);
        self.found.into_iter().collect()
    }
}

fn stamp_value(rest: &[u8]) -> Option<&str> {
    let bytes = &rest[..rest.len().min(MAX_STAMP_VALUE)];
    if !bytes.first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'-' {
        let mut j = i + 1;
        if j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
            j += 1;
            while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            while j < bytes.len() && bytes[j] == b'.' {
                let next = j + 1;
                if next >= bytes.len() || !bytes[next].is_ascii_alphanumeric() {
                    break;
                }
                j = next + 1;
                while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                    j += 1;
                }
            }
            i = j;
        }
    }
    std::str::from_utf8(&bytes[..i]).ok()
}

pub fn authenticate_image(file: &mut File, expect: &ExpectedImage) -> Result<ImageFacts> {
    let identity = file_tx::identity_of(file)?;
    if let Some(volume) = expect.volume
        && identity.volume_serial() != volume
    {
        bail!(
            "the update was staged on a different volume from the executable it replaces, which no atomic replacement can cross"
        );
    }

    file.seek(std::io::SeekFrom::Start(0))
        .context("rewinding the image")?;
    let mut stamps = StampScan::new();
    let mut head = Vec::with_capacity(4096);
    let (measured, digest) =
        crate::file_tx::hash_reader_with(file, "hashed image size overflowed u64", |chunk| {
            stamps.feed(chunk);
            if head.len() < 4096 {
                let room = 4096 - head.len();
                head.extend_from_slice(&chunk[..room.min(chunk.len())]);
            }
            Ok(())
        })
        .context("reading the image")?;
    let sha256 = hex::encode(digest);

    if measured != expect.size {
        bail!(
            "the update is {measured} bytes, not the published {}",
            expect.size
        );
    }
    if sha256 != expect.sha256 {
        bail!(
            "the update's SHA-256 is {sha256}, not the published {}",
            expect.sha256
        );
    }
    verify_pe_shape(&head)?;

    let found = stamps.finish();
    match found.as_slice() {
        [only] if *only == expect.version => {}
        [] => bail!("the update carries no AITIERLIST version stamp at all"),
        [only] => bail!(
            "the update is stamped {only}, not the {} it was published as",
            expect.version
        ),
        many => bail!(
            "the update carries more than one version stamp ({})",
            many.join(", ")
        ),
    }

    Ok(ImageFacts { identity })
}

pub fn verify_pe_shape(head: &[u8]) -> Result<()> {
    const PE_OFFSET_AT: usize = 0x3C;
    if head.len() < PE_OFFSET_AT + 4 || &head[..2] != b"MZ" {
        bail!("the update is not a Windows executable");
    }
    let signature_at = u32::from_le_bytes([
        head[PE_OFFSET_AT],
        head[PE_OFFSET_AT + 1],
        head[PE_OFFSET_AT + 2],
        head[PE_OFFSET_AT + 3],
    ]) as usize;
    let magic_at = signature_at + 4 + 20;
    if magic_at + 2 > head.len() {
        bail!("the update's PE headers are not where a Windows executable keeps them");
    }
    if &head[signature_at..signature_at + 4] != b"PE\0\0" {
        bail!("the update is not a Windows executable");
    }
    let machine = u16::from_le_bytes([head[signature_at + 4], head[signature_at + 5]]);
    const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
    if machine != IMAGE_FILE_MACHINE_AMD64 {
        bail!("the update is not built for x86-64");
    }
    let magic = u16::from_le_bytes([head[magic_at], head[magic_at + 1]]);
    const PE32_PLUS: u16 = 0x20B;
    if magic != PE32_PLUS {
        bail!("the update is not a 64-bit executable");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Control {
    pub schema: u32,
    pub parent_pid: u32,
    pub canonical: String,
    pub canonical_volume: u64,
    pub canonical_id: String,
    pub stage: String,
    pub version: String,
    pub size: u64,
    pub sha256: String,
    pub backup: String,
    pub status: String,
    pub control: String,
    pub helper: String,
    pub helper_volume: u64,
    pub helper_id: String,
    pub relaunch: bool,
}

pub const CONTROL_SCHEMA: u32 = 1;

impl Control {
    fn expected_image(&self) -> ExpectedImage {
        ExpectedImage {
            size: self.size,
            sha256: self.sha256.clone(),
            version: self.version.clone(),
            volume: Some(self.canonical_volume),
        }
    }

    fn canonical_identity(&self) -> Result<FileIdentity> {
        FileIdentity::from_recorded(self.canonical_volume, &self.canonical_id)
            .context("the control record does not describe a file identity")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub version: String,
    pub outcome: String,
    pub detail: String,
    pub relaunched: bool,
    pub backup: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RecordedBackup {
    pub staged_path: String,
    pub staged_volume: u64,
    pub staged_id: String,
    pub canonical_path: String,
    pub canonical_volume: u64,
    pub canonical_id: String,
    pub backup_path: String,
    pub backup_volume: u64,
    pub backup_id: String,
    pub version: String,
    pub phase: BackupPhase,
}

impl RecordedBackup {
    pub fn staged_identity(&self) -> Option<FileIdentity> {
        FileIdentity::from_recorded(self.staged_volume, &self.staged_id)
    }

    pub fn canonical_identity(&self) -> Option<FileIdentity> {
        FileIdentity::from_recorded(self.canonical_volume, &self.canonical_id)
    }

    pub fn backup_identity(&self) -> Option<FileIdentity> {
        FileIdentity::from_recorded(self.backup_volume, &self.backup_id)
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackupPhase {
    Pending,
    Confirmed,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RecordedHelper {
    pub path: String,
    pub volume: u64,
    pub id: String,
}

pub fn backup_record_path(canonical: &Path) -> Result<PathBuf> {
    Ok(state_dir()?.join(format!(
        "replaced-image-{}.json",
        canonical_token(canonical)
    )))
}

pub fn helper_record_path(canonical: &Path) -> Result<PathBuf> {
    Ok(state_dir()?.join(format!("helper-copy-{}.json", canonical_token(canonical))))
}

fn this_canonical() -> Result<PathBuf> {
    let spelling = std::env::current_exe().context("locating the running executable")?;
    match File::open(&spelling) {
        Ok(file) => file_tx::resolved_path_of(&file).or(Ok(spelling)),
        Err(_) => Ok(spelling),
    }
}

pub fn cleanup_replaced_image() {
    let Ok(canonical) = this_canonical() else {
        return;
    };
    cleanup_replaced_image_for(&canonical);
}

fn cleanup_replaced_image_for(canonical: &Path) {
    let Ok(record_path) = backup_record_path(canonical) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&record_path) else {
        return;
    };
    let Ok(record) = serde_json::from_str::<RecordedBackup>(&text) else {
        return;
    };
    match record.phase {
        BackupPhase::Pending => {
            let confirmed = RecordedBackup {
                phase: BackupPhase::Confirmed,
                ..record
            };
            if let Ok(text) = serde_json::to_string_pretty(&confirmed) {
                let _ = file_tx::replace_file_contents(&record_path, text.as_bytes());
            }
        }
        BackupPhase::Confirmed => {
            let Some(identity) = record.backup_identity() else {
                return;
            };
            if file_tx::remove_recorded_object(Path::new(&record.backup_path), identity)
                .unwrap_or(false)
            {
                let _ = std::fs::remove_file(&record_path);
            }
        }
    }
}

pub fn cleanup_helper_copy() {
    let Ok(canonical) = this_canonical() else {
        return;
    };
    cleanup_helper_copy_for(&canonical);
}

fn cleanup_helper_copy_for(canonical: &Path) {
    let Ok(record_path) = helper_record_path(canonical) else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&record_path) else {
        return;
    };
    let Ok(record) = serde_json::from_str::<RecordedHelper>(&text) else {
        return;
    };
    let Some(identity) = FileIdentity::from_recorded(record.volume, &record.id) else {
        return;
    };
    if file_tx::remove_recorded_object(Path::new(&record.path), identity).unwrap_or(false) {
        let _ = std::fs::remove_file(&record_path);
    }
}

pub fn cleanup_previous_update() {
    cleanup_helper_copy();
    cleanup_replaced_image();
}

pub struct StageHandoff<'a> {
    pub stage: &'a mut OwnedStagingFile,
    pub version: &'a str,
    pub size: u64,
    pub sha256: &'a str,
}

pub fn hand_off(staged: StageHandoff<'_>, relaunch: bool) -> Result<()> {
    windows_handoff::hand_off(staged, relaunch)
}

pub const HELPER_ARGUMENT_COUNT: usize = 7;

pub fn helper_launch_requested() -> bool {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    args.len() == HELPER_ARGUMENT_COUNT && args.get(1).is_some_and(|arg| arg == HELPER_SELECTOR)
}

fn parse_inherited_handle(raw: &str) -> Result<usize> {
    if raw.is_empty()
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
        || (raw.len() > 1 && raw.starts_with('0'))
    {
        bail!("an inherited handle value must be a plain decimal number, not {raw:?}");
    }
    let value: usize = raw
        .parse()
        .with_context(|| format!("{raw:?} is not a handle value this process can hold"))?;
    if value == 0 || value == usize::MAX {
        bail!("{raw:?} is not a handle");
    }
    Ok(value)
}

pub fn run_helper() -> u32 {
    windows_handoff::run_helper()
}

fn write_status(path: &Path, status: &Status) {
    let bounded = Status {
        version: bounded_text(&status.version, 64),
        outcome: bounded_text(&status.outcome, 64),
        detail: bounded_text(&status.detail, 2048),
        relaunched: status.relaunched,
        backup: bounded_text(&status.backup, 1024),
    };
    if let Ok(text) = serde_json::to_string_pretty(&bounded) {
        let _ = file_tx::replace_file_contents(path, text.as_bytes());
    }
}

fn bounded_text(text: &str, limit: usize) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if cleaned.chars().count() <= limit {
        return cleaned;
    }
    cleaned.chars().take(limit).collect::<String>() + "…"
}

fn unique_run_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{:08x}-{nonce:032x}-{:016x}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn read_bounded_from(file: File, path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    Read::take(file, limit + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() as u64 > limit {
        bail!("{} is larger than it can legitimately be", path.display());
    }
    Ok(bytes)
}

fn format_recorded_identity(identity: FileIdentity) -> String {
    format!("{}:{}", identity.volume_serial(), identity.file_id_hex())
}

fn parse_recorded_identity(raw: &str) -> Result<FileIdentity> {
    let Some((volume, id)) = raw.split_once(':') else {
        bail!("a recorded file identity must be volume:id, not {raw:?}");
    };
    if volume.is_empty()
        || !volume.bytes().all(|byte| byte.is_ascii_digit())
        || (volume.len() > 1 && volume.starts_with('0'))
    {
        bail!(
            "the volume half of a recorded file identity must be a plain decimal number, not {volume:?}"
        );
    }
    let volume: u64 = volume
        .parse()
        .with_context(|| format!("{volume:?} is not a volume serial this process can record"))?;
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!(
            "the object half of a recorded file identity is not a 32-digit lowercase hex file id"
        );
    }
    FileIdentity::from_recorded(volume, id)
        .context("the object half of a recorded file identity is not a 32-digit hex file id")
}

fn open_recorded_control(path: &Path, expected: FileIdentity) -> Result<Control> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let identity = file_tx::identity_of(&file)?;
    if identity != expected {
        bail!(
            "the handoff record at {} is not the object this helper was launched against",
            path.display()
        );
    }
    let bytes = read_bounded_from(file, path, MAX_CONTROL_BYTES)?;
    let control: Control =
        serde_json::from_slice(&bytes).context("the handoff record is not valid")?;
    if control.schema != CONTROL_SCHEMA {
        bail!("the handoff record is from another version of AITIERLIST");
    }
    Ok(control)
}

#[cfg(windows)]
mod windows_handoff {
    use super::*;
    use std::ffi::c_void;
    use std::io::Write;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use std::ptr;
    use windows_sys::Win32::Foundation::{
        CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, INVALID_HANDLE_VALUE, TRUE,
        WAIT_OBJECT_0,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, SYNCHRONIZE,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_NO_WINDOW, CreateEventW, CreateProcessW, DETACHED_PROCESS,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
        GetProcessId, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, STARTUPINFOEXW, SetEvent,
        TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
    };

    fn is_denied_environment_key(key: &str) -> bool {
        fn has_prefix(key: &str, prefix: &str) -> bool {
            key.as_bytes()
                .get(..prefix.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(prefix.as_bytes()))
        }
        has_prefix(key, "VK_") || has_prefix(key, "WGPU_")
    }

    fn deny_environment(command: &mut Command) -> &mut Command {
        for (key, _) in std::env::vars_os() {
            let denied = key
                .to_str()
                .map(is_denied_environment_key)
                .unwrap_or_else(|| is_denied_environment_key(&key.to_string_lossy()));
            if denied {
                command.env_remove(&key);
            }
        }
        command
    }

    pub(super) struct Owned(HANDLE);

    impl Owned {
        fn get(&self) -> HANDLE {
            self.0
        }

        fn close(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe { CloseHandle(self.0) };
            }
            self.0 = ptr::null_mut();
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            self.close();
        }
    }

    struct Inherited(HANDLE);

    impl Inherited {
        fn from_argument(raw: &std::ffi::OsString, what: &str) -> Result<Self> {
            let text = raw
                .to_str()
                .with_context(|| format!("the {what} handle argument is not text"))?;
            let value = parse_inherited_handle(text)
                .with_context(|| format!("reading the inherited {what} handle"))?;
            Ok(Inherited(value as HANDLE))
        }

        fn get(&self) -> HANDLE {
            self.0
        }

        fn close(&mut self) {
            if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
                unsafe { CloseHandle(self.0) };
            }
            self.0 = ptr::null_mut();
        }
    }

    struct AttributeList {
        storage: Vec<usize>,
        initialized: bool,
    }

    impl AttributeList {
        fn new(count: u32) -> Result<Self> {
            let mut size: usize = 0;
            unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &mut size) };
            if size == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("sizing the update helper's process attribute list");
            }
            let mut list = AttributeList {
                storage: vec![0_usize; size.div_ceil(size_of::<usize>())],
                initialized: false,
            };
            let ok =
                unsafe { InitializeProcThreadAttributeList(list.as_ptr(), count, 0, &mut size) };
            if ok == 0 {
                return Err(std::io::Error::last_os_error())
                    .context("initialising the update helper's process attribute list");
            }
            list.initialized = true;
            Ok(list)
        }

        fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
            self.storage.as_mut_ptr().cast()
        }

        fn set(&mut self, attribute: u32, value: *const c_void, size: usize) -> Result<()> {
            let list = self.as_ptr();
            let ok = unsafe {
                UpdateProcThreadAttribute(
                    list,
                    0,
                    attribute as usize,
                    value,
                    size,
                    ptr::null_mut(),
                    ptr::null(),
                )
            };
            if ok == 0 {
                return Err(std::io::Error::last_os_error())
                    .with_context(|| format!("setting process attribute {attribute:#010x}"));
            }
            Ok(())
        }
    }

    impl Drop for AttributeList {
        fn drop(&mut self) {
            if self.initialized {
                unsafe { DeleteProcThreadAttributeList(self.storage.as_mut_ptr().cast()) };
            }
        }
    }

    fn wide(text: &std::ffi::OsStr) -> Result<Vec<u16>> {
        let mut units: Vec<u16> = text.encode_wide().collect();
        if units.contains(&0) {
            bail!("a Windows name cannot contain an embedded NUL");
        }
        units.push(0);
        Ok(units)
    }

    pub(super) fn create_ready_event() -> Result<Owned> {
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: ptr::null_mut(),
            bInheritHandle: TRUE,
        };
        let handle = unsafe { CreateEventW(&raw const security, 1, 0, ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error())
                .context("creating the update readiness event");
        }
        Ok(Owned(handle))
    }

    fn duplicate_inheritable(
        handle: HANDLE,
        access: u32,
        same_access: bool,
        label: &str,
    ) -> Result<Owned> {
        let mut duplicate: HANDLE = ptr::null_mut();
        let ok = unsafe {
            let process = GetCurrentProcess();
            DuplicateHandle(
                process,
                handle,
                process,
                &mut duplicate,
                if same_access { 0 } else { access },
                TRUE,
                if same_access {
                    DUPLICATE_SAME_ACCESS
                } else {
                    0
                },
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("duplicating the update helper's {label} handle"));
        }
        Ok(Owned(duplicate))
    }

    fn wait_for_readiness(ready: HANDLE, helper: HANDLE, limit: Duration) -> Result<()> {
        let deadline = Instant::now() + limit;
        loop {
            let slice =
                Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now()));
            let outcome = unsafe { WaitForSingleObject(ready, slice.as_millis() as u32) };
            if outcome == WAIT_OBJECT_0 {
                return Ok(());
            }
            let gone = unsafe { WaitForSingleObject(helper, 0) };
            if gone == WAIT_OBJECT_0 {
                bail!("the update helper stopped before it was ready");
            }
            if Instant::now() >= deadline {
                bail!(
                    "the update helper did not report ready within {} seconds",
                    limit.as_secs()
                );
            }
        }
    }

    fn append_argument(argument: &[u16], line: &mut Vec<u16>) -> Result<()> {
        const QUOTE: u16 = b'"' as u16;
        const BACKSLASH: u16 = b'\\' as u16;
        const SPACE: u16 = b' ' as u16;
        const TAB: u16 = b'\t' as u16;
        if argument.contains(&0) {
            bail!("an embedded NUL cannot be carried through a Windows command line");
        }
        let quote =
            argument.is_empty() || argument.iter().any(|&unit| unit == SPACE || unit == TAB);
        if quote {
            line.push(QUOTE);
        }
        let mut backslashes = 0_usize;
        for &unit in argument {
            match unit {
                BACKSLASH => backslashes += 1,
                QUOTE => {
                    line.resize(line.len() + backslashes + 1, BACKSLASH);
                    backslashes = 0;
                }
                _ => backslashes = 0,
            }
            line.push(unit);
        }
        if quote {
            line.resize(line.len() + backslashes, BACKSLASH);
            line.push(QUOTE);
        }
        Ok(())
    }

    fn build_command_line(program: &[u16], arguments: &[Vec<u16>]) -> Result<Vec<u16>> {
        const QUOTE: u16 = b'"' as u16;
        const SPACE: u16 = b' ' as u16;
        if program.contains(&0) || program.contains(&QUOTE) {
            bail!("the update helper's own path cannot carry a NUL or a quote");
        }
        let mut line = vec![QUOTE];
        line.extend_from_slice(program);
        line.push(QUOTE);
        for argument in arguments {
            line.push(SPACE);
            append_argument(argument, &mut line)?;
        }
        if line.len() + 1 > 32_767 {
            bail!("the update helper's command line is past the documented limit");
        }
        line.push(0);
        Ok(line)
    }

    fn wait(handle: HANDLE, limit: Duration) -> Result<()> {
        let milliseconds = limit.as_millis().min(u128::from(u32::MAX - 1)) as u32;
        let outcome = unsafe { WaitForSingleObject(handle, milliseconds) };
        if outcome != WAIT_OBJECT_0 {
            bail!("the wait ended without the expected signal ({outcome:#x})");
        }
        Ok(())
    }

    fn make_helper_copy(exe: &Path, source: &mut File) -> Result<(PathBuf, ImageFacts, File)> {
        source
            .seek(std::io::SeekFrom::Start(0))
            .context("rewinding the running image before copying it")?;
        let mut stage = OwnedStagingFile::create_beside_named(exe, file_tx::helper_leaf_name(exe))?;
        let (copied, digest) = {
            let file = stage.file()?;
            let result = crate::file_tx::hash_reader_with(
                source,
                "hashed helper copy size overflowed u64",
                |chunk| file.write_all(chunk),
            )
            .context("copying the running image")?;
            file.flush().context("flushing the update helper")?;
            file.sync_all().context("syncing the update helper")?;
            result
        };
        let digest = hex::encode(digest);
        let path = stage.path().to_path_buf();
        stage.close_handle();

        let expect = ExpectedImage {
            size: copied,
            sha256: digest,
            version: env!("CARGO_PKG_VERSION").to_string(),
            volume: None,
        };
        let mut pinned = file_tx::open_launch_image(&path)
            .context("re-opening the update helper copy to prove its bytes")?;
        if file_tx::identity_of(&pinned)? != stage.identity() {
            bail!("the update helper copy is no longer the object that was written");
        }
        let facts = authenticate_image(&mut pinned, &expect)
            .context("the update helper copy is not this program's own image")?;
        let _ = stage.keep();
        Ok((path, facts, pinned))
    }

    fn open_running_image() -> Result<(File, PathBuf, FileIdentity)> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        };

        let spelling = std::env::current_exe().context("locating the running executable")?;
        let file = std::fs::OpenOptions::new()
            .access_mode(FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&spelling)
            .with_context(|| format!("opening {}", spelling.display()))?;
        let canonical = file_tx::resolved_path_of(&file)
            .context("resolving the running executable through its own handle")?;
        let identity = file_tx::identity_of(&file)?;
        Ok((file, canonical, identity))
    }

    pub fn hand_off(staged: StageHandoff<'_>, relaunch: bool) -> Result<()> {
        let (mut image, exe, canonical) = open_running_image()?;

        let expect = ExpectedImage {
            size: staged.size,
            sha256: staged.sha256.to_owned(),
            version: staged.version.to_owned(),
            volume: Some(canonical.volume_serial()),
        };
        let stage_path = staged.stage.path().to_path_buf();
        {
            let file = staged.stage.file()?;
            authenticate_image(file, &expect).context("the downloaded update did not verify")?;
            file.sync_all().context("syncing the staged update")?;
        }
        staged.stage.close_handle();
        let mut stage_pin = file_tx::open_launch_image(&stage_path)
            .context("pinning the staged update without write sharing")?;
        authenticate_image(&mut stage_pin, &expect)
            .context("the downloaded update changed before it could be handed off")?;

        let directory = exe
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let backup = directory.join(file_tx::backup_leaf_name(&exe));

        let (helper_path, helper_facts, helper_image) = make_helper_copy(&exe, &mut image)?;
        if let Ok(path) = helper_record_path(&exe) {
            let record = RecordedHelper {
                path: helper_path.to_string_lossy().into_owned(),
                volume: helper_facts.identity.volume_serial(),
                id: helper_facts.identity.file_id_hex(),
            };
            if let Ok(text) = serde_json::to_string_pretty(&record) {
                let _ = file_tx::replace_file_contents(&path, text.as_bytes());
            }
        }

        let state = state_dir()?;
        let run = unique_run_name();
        let control_path = state.join(format!("handoff-{run}.json"));
        let status_path = state.join(format!("handoff-{run}.status.json"));
        let control = Control {
            schema: CONTROL_SCHEMA,
            parent_pid: std::process::id(),
            canonical: exe.to_string_lossy().into_owned(),
            canonical_volume: canonical.volume_serial(),
            canonical_id: canonical.file_id_hex(),
            stage: stage_path.to_string_lossy().into_owned(),
            version: staged.version.to_owned(),
            size: staged.size,
            sha256: staged.sha256.to_owned(),
            backup: backup.to_string_lossy().into_owned(),
            status: status_path.to_string_lossy().into_owned(),
            control: control_path.to_string_lossy().into_owned(),
            helper: helper_path.to_string_lossy().into_owned(),
            helper_volume: helper_facts.identity.volume_serial(),
            helper_id: helper_facts.identity.file_id_hex(),
            relaunch,
        };
        let text = serde_json::to_string_pretty(&control).context("writing the handoff record")?;
        let control_identity = {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&control_path)
                .context("creating the update handoff record")?;
            file.write_all(text.as_bytes())
                .and_then(|()| file.sync_all())
                .context("writing the update handoff record")?;
            file_tx::identity_of(&file)?
        };

        let parent = duplicate_inheritable(
            unsafe { GetCurrentProcess() },
            SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
            false,
            "parent process",
        )?;
        let ready = create_ready_event()?;
        let image_handle =
            duplicate_inheritable(image.as_raw_handle() as HANDLE, 0, true, "running image")?;
        let handle_list = [parent.get(), ready.get(), image_handle.get()];

        let launch_path = file_tx::resolved_path_of(&helper_image)
            .context("resolving the update helper through its authenticated handle")?;
        let program: Vec<u16> = launch_path.as_os_str().encode_wide().collect();
        let application = wide(launch_path.as_os_str())?;
        let arguments: Vec<Vec<u16>> = vec![
            std::ffi::OsStr::new(HELPER_SELECTOR)
                .encode_wide()
                .collect(),
            control_path.as_os_str().encode_wide().collect(),
            std::ffi::OsStr::new(&format!("{}", parent.get() as usize))
                .encode_wide()
                .collect(),
            std::ffi::OsStr::new(&format!("{}", ready.get() as usize))
                .encode_wide()
                .collect(),
            std::ffi::OsStr::new(&format!("{}", image_handle.get() as usize))
                .encode_wide()
                .collect(),
            std::ffi::OsStr::new(&format_recorded_identity(control_identity))
                .encode_wide()
                .collect(),
        ];
        let mut command_line = build_command_line(&program, &arguments)?;

        let mut attributes = AttributeList::new(1)?;
        attributes.set(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            handle_list.as_ptr().cast(),
            size_of_val(&handle_list),
        )?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = attributes.as_ptr();

        let mut information = PROCESS_INFORMATION::default();
        let created = unsafe {
            CreateProcessW(
                application.as_ptr(),
                command_line.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                TRUE,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW | DETACHED_PROCESS,
                ptr::null(),
                ptr::null(),
                &raw const startup.StartupInfo,
                &mut information,
            )
        };
        if created == 0 {
            let error = std::io::Error::last_os_error();
            retire_failed_attempt(
                &control_path,
                control_identity,
                &helper_path,
                &helper_facts,
                &exe,
            );
            return Err(error).context("starting the update helper");
        }
        let helper_process = Owned(information.hProcess);
        let _helper_thread = Owned(information.hThread);

        match process_image_path(helper_process.get()).and_then(|path| {
            file_tx::existing_identity(&path)?
                .context("the helper process image is no longer at its own path")
        }) {
            Ok(launched) if launched == helper_facts.identity => {}
            _ => {
                unsafe { TerminateProcess(helper_process.get(), HELPER_FAILURE_EXIT) };
                unsafe { WaitForSingleObject(helper_process.get(), 5_000) };
                retire_failed_attempt(
                    &control_path,
                    control_identity,
                    &helper_path,
                    &helper_facts,
                    &exe,
                );
                bail!("the process that started is not the helper copy that was authenticated");
            }
        }
        drop(helper_image);

        let mut parent = parent;
        let mut image_handle = image_handle;
        parent.close();
        image_handle.close();
        drop(image);

        match wait_for_readiness(ready.get(), helper_process.get(), READY_WAIT) {
            Ok(()) => {}
            Err(error) => {
                unsafe { TerminateProcess(helper_process.get(), HELPER_FAILURE_EXIT) };
                unsafe { WaitForSingleObject(helper_process.get(), 5_000) };
                retire_failed_attempt(
                    &control_path,
                    control_identity,
                    &helper_path,
                    &helper_facts,
                    &exe,
                );
                return Err(error).context(
                    "the update helper did not come up, so nothing was installed and the download was kept",
                );
            }
        }

        staged.stage.mark_kept();
        Ok(())
    }

    fn retire_failed_attempt(
        control_path: &Path,
        control_identity: FileIdentity,
        helper_path: &Path,
        helper_facts: &ImageFacts,
        canonical: &Path,
    ) {
        let _ = file_tx::remove_recorded_object(control_path, control_identity);
        if file_tx::remove_recorded_object(helper_path, helper_facts.identity).unwrap_or(false)
            && let Ok(record) = helper_record_path(canonical)
            && let Ok(Some(identity)) = file_tx::existing_identity(&record)
        {
            let _ = file_tx::remove_recorded_object(&record, identity);
        }
    }

    pub(super) struct Capabilities {
        parent: Inherited,
        ready: Inherited,
        image: Inherited,
    }

    impl Capabilities {
        pub(super) fn from_arguments(arguments: &[std::ffi::OsString]) -> Result<Self> {
            if arguments.len() != HELPER_ARGUMENT_COUNT {
                bail!(
                    "the update helper was started with {} arguments, not the {} a handoff carries",
                    arguments.len(),
                    HELPER_ARGUMENT_COUNT
                );
            }
            let parent = Inherited::from_argument(&arguments[3], "parent process")?;
            let ready = Inherited::from_argument(&arguments[4], "readiness event")?;
            let image = Inherited::from_argument(&arguments[5], "running image")?;
            if parent.get() == ready.get()
                || parent.get() == image.get()
                || ready.get() == image.get()
            {
                bail!("the update helper was handed the same handle twice");
            }
            Ok(Capabilities {
                parent,
                ready,
                image,
            })
        }

        fn close(&mut self) {
            self.parent.close();
            self.ready.close();
            self.image.close();
        }
    }

    fn process_image_path(process: HANDLE) -> Result<PathBuf> {
        let mut buffer = vec![0_u16; 32 * 1024];
        let mut length = buffer.len() as u32;
        let ok = unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                buffer.as_mut_ptr(),
                &raw mut length,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error())
                .context("reading a process's own image path");
        }
        Ok(PathBuf::from(std::ffi::OsString::from_wide(
            &buffer[..length as usize],
        )))
    }

    fn authenticate_capabilities(control: &Control, capabilities: &Capabilities) -> Result<()> {
        let pid = unsafe { GetProcessId(capabilities.parent.get()) };
        if pid == 0 {
            return Err(std::io::Error::last_os_error())
                .context("the inherited parent handle is not a process");
        }
        if pid != control.parent_pid {
            bail!("the inherited parent handle is not the process the handoff record describes");
        }

        let expected = control.canonical_identity()?;
        let running = process_image_path(capabilities.parent.get())?;
        let running = file_tx::existing_identity(&running)?
            .context("the parent process's image is no longer at its own path")?;
        if running != expected {
            bail!("the process that asked for this update is not the AITIERLIST it claimed to be");
        }

        let image = unsafe_image_file(capabilities.image.get());
        let derived =
            file_tx::resolved_path_of(&image).context("resolving the inherited image handle")?;
        let derived_identity = file_tx::identity_of(&image)?;
        if derived_identity != expected {
            bail!("the inherited image handle is not the executable this update replaces");
        }
        let canonical = file_tx::existing_identity(Path::new(&control.canonical))?
            .context("the executable this update replaces is no longer there")?;
        if canonical != expected {
            bail!("the executable this update replaces is not the one the request described");
        }
        let named = file_tx::existing_identity(&derived)?
            .context("the path the inherited image handle resolves to is no longer there")?;
        if named != expected {
            bail!("the running image no longer answers to the path it resolves to");
        }
        Ok(())
    }

    fn unsafe_image_file(handle: HANDLE) -> std::mem::ManuallyDrop<File> {
        use std::os::windows::io::FromRawHandle;
        std::mem::ManuallyDrop::new(unsafe { File::from_raw_handle(handle) })
    }

    fn authenticate_stage(control: &Control) -> Result<(File, ImageFacts)> {
        let mut file = file_tx::open_pinned(Path::new(&control.stage))
            .with_context(|| format!("opening {}", control.stage))?;
        let facts = authenticate_image(&mut file, &control.expected_image())?;
        Ok((file, facts))
    }

    fn authenticate_helper_image(control: &Control) -> Result<()> {
        let expected = FileIdentity::from_recorded(control.helper_volume, &control.helper_id)
            .context("the control record does not describe the helper")?;
        let path = process_image_path(unsafe { GetCurrentProcess() })?;
        let identity = file_tx::existing_identity(&path)?
            .context("this helper's image is no longer at its own path")?;
        if identity != expected {
            bail!("this helper is not the copy that was authenticated");
        }
        Ok(())
    }

    pub(super) fn verify_or_recover_failed_replace(record: &RecordedBackup) -> Result<()> {
        let canonical = Path::new(&record.canonical_path);
        let expected = record
            .canonical_identity()
            .context("the recorded canonical identity is malformed")?;
        let staged = record
            .staged_identity()
            .context("the recorded staged identity is malformed")?;
        let backup = Path::new(&record.backup_path);

        match file_tx::existing_identity(canonical) {
            Ok(Some(identity)) if identity == expected || identity == staged => return Ok(()),
            Ok(Some(_)) => {
                bail!("the canonical executable changed identity during a failed replacement")
            }
            Ok(None) => {}
            Err(error) => {
                return Err(error).context(
                    "the canonical executable could not be identified after a failed replacement",
                );
            }
        }

        match file_tx::move_recorded_object_no_replace(backup, expected, canonical) {
            Ok(true) => {}
            Ok(false) => {
                bail!(
                    "the failed replacement left the canonical name empty, but its recovery name is absent or no longer identifies the pre-call executable"
                )
            }
            Err(error) => {
                return Err(error).context(
                    "the failed replacement left the canonical name empty and recovery failed",
                );
            }
        }
        match file_tx::existing_identity(canonical)? {
            Some(identity) if identity == expected => Ok(()),
            Some(_) => bail!("the recovered canonical name identifies a different object"),
            None => bail!("the canonical executable is still absent after recovery"),
        }
    }

    pub fn run_helper() -> u32 {
        match helper_body() {
            Ok(()) => 0,
            Err(_) => HELPER_FAILURE_EXIT,
        }
    }

    fn helper_body() -> Result<()> {
        let arguments: Vec<std::ffi::OsString> = std::env::args_os().collect();
        let mut capabilities = Capabilities::from_arguments(&arguments)?;
        let control_path = PathBuf::from(&arguments[2]);
        let expected_control = arguments[6]
            .to_str()
            .context("the handoff record identity is not text")
            .and_then(parse_recorded_identity)?;
        let control = open_recorded_control(&control_path, expected_control)?;
        let status_path = PathBuf::from(&control.status);

        let _lock = match UpdateLock::acquire_waiting(Path::new(&control.canonical), LOCK_WAIT) {
            Ok(lock) => lock,
            Err(error) => {
                report(&status_path, &control, "lock-refused", &error, false);
                return Err(error);
            }
        };

        if let Err(error) = authenticate_capabilities(&control, &capabilities) {
            report(&status_path, &control, "parent-unverified", &error, false);
            return Err(error);
        }
        if let Err(error) = authenticate_helper_image(&control) {
            report(&status_path, &control, "helper-unverified", &error, false);
            return Err(error);
        }
        let stage = match authenticate_stage(&control) {
            Ok((file, _)) => file,
            Err(error) => {
                report(&status_path, &control, "stage-unverified", &error, false);
                return Err(error);
            }
        };

        if unsafe { SetEvent(capabilities.ready.get()) } == 0 {
            let error = anyhow::Error::from(std::io::Error::last_os_error())
                .context("signalling the update readiness event");
            report(&status_path, &control, "not-ready", &error, false);
            return Err(error);
        }

        capabilities.image.close();

        if let Err(error) = wait(capabilities.parent.get(), PARENT_WAIT) {
            let error = error.context(
                "AITIERLIST did not exit, so the update was not installed and the download was kept",
            );
            report(
                &status_path,
                &control,
                "parent-still-running",
                &error,
                false,
            );
            return Err(error);
        }

        capabilities.close();

        let outcome = commit(&control, &status_path, stage);
        let _ = file_tx::remove_recorded_object(&control_path, expected_control);
        outcome
    }

    fn commit(control: &Control, status_path: &Path, mut stage: File) -> Result<()> {
        if let Err(error) = authenticate_image(&mut stage, &control.expected_image()) {
            report(status_path, control, "stage-unverified", &error, false);
            return Err(error);
        }
        let canonical = PathBuf::from(&control.canonical);
        let previous = match file_tx::existing_identity(&canonical) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                let error = anyhow::anyhow!("the executable to replace is no longer there");
                report(status_path, control, "target-missing", &error, false);
                return Err(error);
            }
            Err(error) => {
                report(status_path, control, "target-unreadable", &error, false);
                return Err(error);
            }
        };
        let backup = PathBuf::from(&control.backup);
        let staged_identity = file_tx::identity_of(&stage)?;
        let directory = canonical
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let target_leaf = canonical
            .file_name()
            .context("the executable to replace has no file name")?;
        let backup_leaf = backup.file_name().context("the backup has no file name")?;
        let parent = file_tx::lease_directory(directory)?;

        let record = match record_backup(control, previous, staged_identity) {
            Ok(r) => r,
            Err(error) => {
                report(status_path, control, "backup-record-failed", &error, false);
                return Err(error);
            }
        };

        let canonical_handle = match (|| -> Result<File> {
            let handle = file_tx::open_no_follow(&canonical, FILE_SHARE_READ | FILE_SHARE_DELETE)?;
            if file_tx::identity_of(&handle)? != previous
                || previous != control.canonical_identity()?
            {
                bail!("the canonical does not identify the executable authorized for replacement");
            }
            Ok(handle)
        })() {
            Ok(handle) => handle,
            Err(error) => {
                std::fs::remove_file(backup_record_path(&canonical)?).with_context(|| {
                    format!("removing the pending backup record after: {error:#}")
                })?;
                report(status_path, control, "target-unverified", &error, false);
                return Err(error);
            }
        };
        if let Err(error) = file_tx::link_pinned(&canonical_handle, &parent, backup_leaf) {
            std::fs::remove_file(backup_record_path(&canonical)?)
                .with_context(|| format!("removing the pending backup record after: {error:#}"))?;
            report(status_path, control, "link-failed", &error, false);
            return Err(error);
        }

        let replacement = file_tx::rename_pinned_replace(&stage, &parent, target_leaf);
        drop(canonical_handle);
        if let Err(error) = replacement {
            let (outcome, error) = match verify_or_recover_failed_replace(&record) {
                Ok(()) => ("not-installed", error),
                Err(recovery) => (
                    "install-unusable",
                    error.context(format!(
                        "the failed install could not restore its pre-call state: {recovery:#}"
                    )),
                ),
            };
            report(status_path, control, outcome, &error, false);
            return Err(error);
        }

        if !control.relaunch {
            write_status(
                status_path,
                &Status {
                    version: control.version.clone(),
                    outcome: "installed".into(),
                    detail: "installed; no relaunch was asked for".into(),
                    relaunched: false,
                    backup: control.backup.clone(),
                },
            );
            return Ok(());
        }

        let canonical_program = canonical.clone();
        let mut command = Command::new(canonical_program);
        command.creation_flags(CREATE_NO_WINDOW);
        deny_environment(&mut command);
        match command.spawn() {
            Ok(_) => {
                write_status(
                    status_path,
                    &Status {
                        version: control.version.clone(),
                        outcome: "installed".into(),
                        detail: "installed and relaunched".into(),
                        relaunched: true,
                        backup: control.backup.clone(),
                    },
                );
                Ok(())
            }
            Err(error) => {
                let error = anyhow::Error::from(error)
                    .context("the updated AITIERLIST could not be started, so it was rolled back");
                let restored = restore(&canonical, &backup, previous);
                let relaunched = restored.is_ok() && {
                    let old_program = canonical.clone();
                    let mut retry = Command::new(old_program);
                    retry.creation_flags(CREATE_NO_WINDOW);
                    deny_environment(&mut retry);
                    retry.spawn().is_ok()
                };
                report(
                    status_path,
                    control,
                    if restored.is_ok() {
                        "rolled-back"
                    } else {
                        "install-unusable"
                    },
                    &error,
                    relaunched,
                );
                Err(error)
            }
        }
    }

    fn restore(canonical: &Path, backup: &Path, previous: FileIdentity) -> Result<()> {
        let backup_file = file_tx::open_pinned(backup)?;
        if file_tx::identity_of(&backup_file)? != previous {
            bail!("the recorded backup is not the image this update displaced");
        }
        let directory = canonical
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let target_leaf = canonical
            .file_name()
            .context("the executable to restore has no file name")?;
        let parent = file_tx::lease_directory(directory)?;
        let displaced = file_tx::existing_identity(canonical)?
            .context("the executable to roll back is no longer there")?;

        let failed_leaf = file_tx::backup_leaf_name(canonical);
        let failed = directory.join(&failed_leaf);
        let record = RecordedBackup {
            staged_path: backup.to_string_lossy().into_owned(),
            staged_volume: previous.volume_serial(),
            staged_id: previous.file_id_hex(),
            canonical_path: canonical.to_string_lossy().into_owned(),
            canonical_volume: displaced.volume_serial(),
            canonical_id: displaced.file_id_hex(),
            backup_path: failed.to_string_lossy().into_owned(),
            backup_volume: displaced.volume_serial(),
            backup_id: displaced.file_id_hex(),
            version: "rollback".to_string(),
            phase: BackupPhase::Pending,
        };
        let text =
            serde_json::to_string_pretty(&record).context("serializing the backup record")?;
        let record_path = backup_record_path(canonical)?;
        file_tx::replace_file_contents(&record_path, text.as_bytes())
            .context("writing the backup record")?;
        let canonical_handle = match (|| -> Result<File> {
            let handle = file_tx::open_no_follow(canonical, FILE_SHARE_READ | FILE_SHARE_DELETE)?;
            if Some(file_tx::identity_of(&handle)?) != record.canonical_identity() {
                bail!("the canonical does not identify the executable authorized for rollback");
            }
            file_tx::link_pinned(&handle, &parent, &failed_leaf)?;
            Ok(handle)
        })() {
            Ok(handle) => handle,
            Err(error) => {
                std::fs::remove_file(&record_path).with_context(|| {
                    format!("removing the pending backup record after: {error:#}")
                })?;
                return Err(error);
            }
        };
        let replacement = file_tx::rename_pinned_replace(&backup_file, &parent, target_leaf);
        drop(canonical_handle);
        if let Err(error) = replacement {
            return match verify_or_recover_failed_replace(&record) {
                Ok(()) => Err(error),
                Err(recovery) => Err(error.context(format!(
                    "the failed rollback could not restore its pre-call state: {recovery:#}"
                ))),
            };
        }
        if file_tx::identity_of(&backup_file)? != previous {
            bail!("the rollback did not restore the image this update displaced");
        }
        Ok(())
    }

    fn record_backup(
        control: &Control,
        previous: FileIdentity,
        staged: FileIdentity,
    ) -> Result<RecordedBackup> {
        let path = backup_record_path(Path::new(&control.canonical))?;
        let record = RecordedBackup {
            staged_path: control.stage.clone(),
            staged_volume: staged.volume_serial(),
            staged_id: staged.file_id_hex(),
            canonical_path: control.canonical.clone(),
            canonical_volume: previous.volume_serial(),
            canonical_id: previous.file_id_hex(),
            backup_path: control.backup.clone(),
            backup_volume: previous.volume_serial(),
            backup_id: previous.file_id_hex(),
            version: control.version.clone(),
            phase: BackupPhase::Pending,
        };
        let text =
            serde_json::to_string_pretty(&record).context("serializing the backup record")?;
        file_tx::replace_file_contents(&path, text.as_bytes())
            .context("writing the backup record")?;
        Ok(record)
    }

    fn report(
        status_path: &Path,
        control: &Control,
        outcome: &str,
        error: &anyhow::Error,
        relaunched: bool,
    ) {
        write_status(
            status_path,
            &Status {
                version: control.version.clone(),
                outcome: outcome.to_owned(),
                detail: format!("{error:#}"),
                relaunched,
                backup: control.backup.clone(),
            },
        );
    }
}
