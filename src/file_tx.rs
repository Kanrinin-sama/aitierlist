use anyhow::{Context, Result, bail};
use sha2::Digest;
use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(windows)]
use std::mem::{offset_of, size_of};
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_INVALID_PARAMETER, ERROR_NOT_SUPPORTED, HANDLE,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, DELETE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_TRAVERSE, FILE_WRITE_ATTRIBUTES, FILE_WRITE_DATA, FileDispositionInfo,
    FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    GetVolumeInformationByHandleW, SYNCHRONIZE, SetFileInformationByHandle,
};

#[cfg(windows)]
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_INFORMATION_CLASS, FILE_RENAME_INFORMATION, FILE_RENAME_POSIX_SEMANTICS,
    FILE_RENAME_REPLACE_IF_EXISTS, FileRenameInformation, FileRenameInformationEx,
    NtSetInformationFile,
};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    NTSTATUS, RtlNtStatusToDosError, STATUS_INVALID_INFO_CLASS, STATUS_NOT_IMPLEMENTED,
    STATUS_NOT_SUPPORTED, STATUS_OBJECT_NAME_COLLISION,
};
#[cfg(windows)]
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

pub fn is_indirect(path: &Path) -> std::io::Result<bool> {
    use std::os::windows::fs::MetadataExt;
    Ok(std::fs::symlink_metadata(path)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    volume: u64,
    id: u128,
}

impl FileIdentity {
    pub fn volume_serial(&self) -> u64 {
        self.volume
    }

    pub fn file_id_hex(&self) -> String {
        format!("{:032x}", self.id)
    }

    pub fn from_recorded(volume: u64, id_hex: &str) -> Option<Self> {
        if id_hex.len() != 32 || !id_hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(FileIdentity {
            volume,
            id: u128::from_str_radix(id_hex, 16).ok()?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceFingerprint {
    identity: FileIdentity,
    size: u64,
    written_at_nanos: u128,
}

impl SourceFingerprint {
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub fn size(&self) -> u64 {
        self.size
    }
}

#[derive(Debug)]
pub struct SourceLease {
    file: File,
    path: PathBuf,
    resolved: PathBuf,
    fingerprint: SourceFingerprint,
}

impl SourceLease {
    pub fn fingerprint(&self) -> SourceFingerprint {
        self.fingerprint
    }

    pub fn identity(&self) -> FileIdentity {
        self.fingerprint.identity
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn resolved_path(&self) -> &Path {
        &self.resolved
    }

    pub fn file(&self) -> &File {
        &self.file
    }
}

pub fn resolved_path_of(file: &File) -> std::io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_NAME_NORMALIZED, GetFinalPathNameByHandleW,
    };

    const VOLUME_NAME_DOS: u32 = 0x0;
    let handle = file.as_raw_handle() as HANDLE;
    let needed = unsafe {
        GetFinalPathNameByHandleW(
            handle,
            std::ptr::null_mut(),
            0,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    };
    if needed == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut buffer = vec![0_u16; needed as usize + 1];
    let written = unsafe {
        GetFinalPathNameByHandleW(
            handle,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            FILE_NAME_NORMALIZED | VOLUME_NAME_DOS,
        )
    };
    if written == 0 || written as usize >= buffer.len() {
        return Err(std::io::Error::last_os_error());
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..written as usize],
    )))
}

#[derive(Debug)]
pub struct DirectoryLease {
    file: File,
    path: PathBuf,
    resolved: PathBuf,
}

impl DirectoryLease {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn resolved_path(&self) -> &Path {
        &self.resolved
    }

    pub fn file(&self) -> &File {
        &self.file
    }
}

#[cfg(windows)]
fn handle_information(file: &File) -> Result<BY_HANDLE_FILE_INFORMATION> {
    let mut information = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    let ok = unsafe {
        GetFileInformationByHandle(file.as_raw_handle() as HANDLE, information.as_mut_ptr())
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("querying a Windows file handle");
    }
    Ok(unsafe { information.assume_init() })
}

#[cfg(windows)]
fn filesystem_name(file: &File) -> Result<String> {
    use std::os::windows::ffi::OsStringExt;
    let mut name = [0_u16; 64];
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle() as HANDLE,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error())
            .context("asking which filesystem a file is stored on");
    }
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    Ok(OsString::from_wide(&name[..end])
        .to_string_lossy()
        .into_owned())
}

#[cfg(windows)]
const LEGACY_ID_FILESYSTEMS: &[&str] = &["NTFS", "FAT", "FAT12", "FAT16", "FAT32", "exFAT"];

pub fn identity_of(file: &File) -> Result<FileIdentity> {
    let handle = file.as_raw_handle() as HANDLE;
    let mut wide = std::mem::MaybeUninit::<FILE_ID_INFO>::uninit();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            wide.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok != 0 {
        let wide = unsafe { wide.assume_init() };
        let id = u128::from_le_bytes(wide.FileId.Identifier);
        if id == 0 {
            bail!("this filesystem reported an all-zero file identity, which identifies nothing");
        }
        return Ok(FileIdentity {
            volume: wide.VolumeSerialNumber,
            id,
        });
    }

    let refusal = std::io::Error::last_os_error();
    let unsupported = matches!(
        refusal.raw_os_error().map(|code| code as u32),
        Some(ERROR_INVALID_PARAMETER) | Some(ERROR_NOT_SUPPORTED)
    );
    if !unsupported {
        return Err(refusal).context("reading a file's volume-wide identity");
    }
    let filesystem = filesystem_name(file)?;
    if !LEGACY_ID_FILESYSTEMS
        .iter()
        .any(|known| filesystem.eq_ignore_ascii_case(known))
    {
        bail!(
            "{filesystem} does not report a file identity this program can rely on, \
             so nothing here can be proven to be the file it claims to be"
        );
    }
    let information = handle_information(file)?;
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    if index == 0 {
        bail!("this filesystem does not report a stable file identity");
    }
    Ok(FileIdentity {
        volume: u64::from(information.dwVolumeSerialNumber),
        id: u128::from(index),
    })
}

fn fingerprint_of(file: &File) -> Result<SourceFingerprint> {
    let metadata = file.metadata().context("reading source file metadata")?;
    let written_at_nanos = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    Ok(SourceFingerprint {
        identity: identity_of(file)?,
        size: metadata.len(),
        written_at_nanos,
    })
}

fn open_no_follow(path: &Path, share_mode: u32) -> std::io::Result<File> {
    open_no_follow_access(
        path,
        FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        share_mode,
    )
}

#[cfg(windows)]
fn open_no_follow_access(path: &Path, access: u32, share_mode: u32) -> std::io::Result<File> {
    OpenOptions::new()
        .access_mode(access)
        .share_mode(share_mode)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

const SHARE_READERS_ONLY: u32 = FILE_SHARE_READ;

pub fn open_source(path: &Path) -> Result<SourceLease> {
    let file = open_no_follow(path, SHARE_READERS_ONLY)
        .with_context(|| format!("opening the source file {}", path.display()))?;
    reject_indirection(&file, path)?;
    let fingerprint = fingerprint_of(&file)?;
    let resolved = resolved_path_of(&file).with_context(|| {
        format!(
            "resolving {} to a name a child process can open without walking the path again",
            path.display()
        )
    })?;
    Ok(SourceLease {
        file,
        path: path.to_path_buf(),
        resolved,
        fingerprint,
    })
}

fn reject_indirection(file: &File, path: &Path) -> Result<()> {
    let information = handle_information(file)?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("{} is a reparse point, not a file", path.display());
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
        bail!("{} is a directory, not a file", path.display());
    }
    Ok(())
}

pub fn probe_source(path: &Path) -> Result<SourceFingerprint> {
    Ok(open_source(path)?.fingerprint())
}

pub fn reacquire_source(path: &Path, expected: Option<SourceFingerprint>) -> Result<SourceLease> {
    let lease = open_source(path)?;
    if let Some(expected) = expected
        && lease.fingerprint != expected
    {
        bail!(
            "{} is no longer the file that was inspected - it was replaced, \
             overwritten or moved since it was added; remove it and add it again",
            path.display()
        );
    }
    Ok(lease)
}

pub fn open_pinned(path: &Path) -> Result<File> {
    let file = open_no_follow_access(
        path,
        FILE_READ_DATA | FILE_READ_ATTRIBUTES | DELETE | SYNCHRONIZE,
        FILE_SHARE_READ,
    )
    .with_context(|| format!("opening {} without write sharing", path.display()))?;
    reject_indirection(&file, path)?;
    Ok(file)
}

pub fn open_launch_image(path: &Path) -> Result<File> {
    let file = open_no_follow_access(
        path,
        FILE_READ_DATA | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_DELETE,
    )
    .with_context(|| format!("opening {} as a launch image", path.display()))?;
    reject_indirection(&file, path)?;
    Ok(file)
}

pub fn rename_pinned(file: &File, directory: &DirectoryLease, leaf: &OsStr) -> Result<()> {
    rename_relative(file, directory.file(), leaf).map_err(Into::into)
}

pub fn existing_identity(path: &Path) -> Result<Option<FileIdentity>> {
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
    match open_no_follow(path, share) {
        Ok(file) => Ok(Some(identity_of(&file)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("inspecting {}", path.display())),
    }
}

pub fn identity_pair(path: &Path) -> Option<(u64, u128)> {
    existing_identity(path)
        .ok()
        .flatten()
        .map(|identity| (identity.volume, identity.id))
}

pub fn output_blocker(output: &Path, source: Option<FileIdentity>) -> Option<String> {
    let existing = existing_identity(output).ok().flatten()?;
    if Some(existing) == source {
        return Some(format!(
            "the output path \"{}\" is the source file itself - encoding it would destroy the input; choose another name",
            output.display()
        ));
    }
    Some(format!(
        "\"{}\" already exists - AITIERLIST never overwrites an existing file; choose another name",
        output.display()
    ))
}

fn open_directory(path: &Path) -> Result<File> {
    let file = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY | FILE_TRAVERSE | FILE_READ_ATTRIBUTES | SYNCHRONIZE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .with_context(|| format!("opening the output directory {}", path.display()))?;
    let information = handle_information(&file)?;
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        bail!("the output directory {} is a reparse point", path.display());
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        bail!("{} is not a directory", path.display());
    }
    Ok(file)
}

pub fn lease_directory(path: &Path) -> Result<DirectoryLease> {
    let file = open_directory(path)?;
    let resolved = resolved_path_of(&file)
        .with_context(|| format!("resolving the directory {}", path.display()))?;
    Ok(DirectoryLease {
        file,
        path: path.to_path_buf(),
        resolved,
    })
}

static STAGE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn stage_leaf_name(destination: &Path) -> OsString {
    unique_leaf_name(".aitierlist-stage-", destination)
}

#[derive(Debug)]
pub struct OutputTransaction {
    parent: DirectoryLease,
    stage_path: PathBuf,
    stage: Option<File>,
    final_path: PathBuf,
    final_leaf: OsString,
    committed: bool,
    preserve_stage: bool,
}

impl OutputTransaction {
    pub fn begin(final_path: &Path, forbidden: &[FileIdentity]) -> Result<Self> {
        let directory = final_path.parent().filter(|p| !p.as_os_str().is_empty());
        let directory = directory.unwrap_or_else(|| Path::new("."));
        let final_leaf = final_path
            .file_name()
            .with_context(|| format!("{} has no file name", final_path.display()))?
            .to_os_string();

        let parent = lease_directory(directory)?;
        let final_child = parent.resolved.join(&final_leaf);

        if let Some(existing) = existing_identity(&final_child)? {
            if forbidden.contains(&existing) {
                bail!(
                    "the output path {} is the source file itself; encoding it would destroy the input",
                    final_path.display()
                );
            }
            bail!(
                "{} already exists; AITIERLIST never overwrites an existing file",
                final_path.display()
            );
        }

        for _ in 0..256 {
            let stage_path = parent.resolved.join(stage_leaf_name(final_path));
            match create_owned_stage(&stage_path) {
                Ok(stage) => {
                    let stage = identify_owned_stage(stage)?.0;
                    return Ok(OutputTransaction {
                        parent,
                        stage_path,
                        stage: Some(stage),
                        final_path: final_path.to_path_buf(),
                        final_leaf,
                        committed: false,
                        preserve_stage: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("creating a staging file in {}", parent.path.display())
                    });
                }
            }
        }
        bail!(
            "could not reserve a staging file beside {}",
            final_path.display()
        )
    }

    pub fn stage_path(&self) -> &Path {
        &self.stage_path
    }

    pub fn stage_len(&self) -> Result<u64> {
        let stage = self.stage.as_ref().context("the staging file is closed")?;
        Ok(stage
            .metadata()
            .context("measuring the staged output")?
            .len())
    }

    pub fn sync(&self) -> Result<()> {
        let stage = self.stage.as_ref().context("the staging file is closed")?;
        stage.sync_all().context("syncing the staged output")
    }

    pub fn commit(mut self) -> Result<PathBuf> {
        self.sync()?;
        self.publish()?;
        self.committed = true;
        Ok(self.final_path.clone())
    }

    fn publish(&mut self) -> Result<()> {
        let stage = self.stage.as_ref().context("the staging file is closed")?;
        match rename_relative(stage, &self.parent.file, &self.final_leaf) {
            Ok(()) => {}
            Err(e) if is_collision(&e) => {
                return Err(e).with_context(|| {
                    format!(
                        "{} appeared while the encode was running and was left untouched",
                        self.final_path.display()
                    )
                });
            }
            Err(e) => {
                self.preserve_stage = true;
                return Err(e).context(PreservedStage {
                    path: self.stage_path.clone(),
                });
            }
        }
        self.committed = true;
        Ok(())
    }

    fn remove_owned_stage(&mut self) {
        if let Some(stage) = self.stage.take() {
            let _ = delete_by_handle(&stage);
        }
    }
}

impl Drop for OutputTransaction {
    fn drop(&mut self) {
        if self.committed || self.preserve_stage {
            return;
        }
        self.remove_owned_stage();
    }
}

#[derive(Debug)]
pub struct PreservedStage {
    pub path: PathBuf,
}

impl std::fmt::Display for PreservedStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "publishing the encode's output failed (the encoded file is still at {} and nothing at the destination was touched)",
            self.path.display()
        )
    }
}

impl std::error::Error for PreservedStage {}

fn is_collision(error: &std::io::Error) -> bool {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return true;
    }
    matches!(
        error.raw_os_error().map(|code| code as u32),
        Some(ERROR_ALREADY_EXISTS) | Some(ERROR_FILE_EXISTS)
    )
}

fn create_owned_stage(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(
            DELETE
                | SYNCHRONIZE
                | FILE_READ_DATA
                | FILE_WRITE_DATA
                | FILE_READ_ATTRIBUTES
                | FILE_WRITE_ATTRIBUTES,
        )
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .open(path)
}

fn identify_owned_stage(stage: File) -> Result<(File, FileIdentity)> {
    match identity_of(&stage) {
        Ok(identity) => Ok((stage, identity)),
        Err(identity_error) => {
            #[cfg(windows)]
            {
                if let Err(cleanup_error) = delete_by_handle(&stage) {
                    return Err(identity_error).context(format!(
                        "identifying a newly-created staging file; disposing of it by handle also failed: {cleanup_error}"
                    ));
                }
            }
            Err(identity_error).context("identifying a newly-created staging file")
        }
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenameMode {
    NoReplace,
    Replace,
    LegacyNoReplace,
    LegacyReplace,
}

#[cfg(windows)]
impl RenameMode {
    fn class(self) -> FILE_INFORMATION_CLASS {
        match self {
            RenameMode::NoReplace | RenameMode::Replace => FileRenameInformationEx,
            RenameMode::LegacyNoReplace | RenameMode::LegacyReplace => FileRenameInformation,
        }
    }

    fn flags(self) -> Option<u32> {
        match self {
            RenameMode::NoReplace => Some(FILE_RENAME_POSIX_SEMANTICS),
            RenameMode::Replace => {
                Some(FILE_RENAME_REPLACE_IF_EXISTS | FILE_RENAME_POSIX_SEMANTICS)
            }
            RenameMode::LegacyNoReplace | RenameMode::LegacyReplace => None,
        }
    }

    fn replace_if_exists(self) -> Option<bool> {
        match self {
            RenameMode::NoReplace | RenameMode::Replace => None,
            RenameMode::LegacyNoReplace => Some(false),
            RenameMode::LegacyReplace => Some(true),
        }
    }

    fn legacy(self) -> Self {
        match self {
            RenameMode::NoReplace | RenameMode::LegacyNoReplace => RenameMode::LegacyNoReplace,
            RenameMode::Replace | RenameMode::LegacyReplace => RenameMode::LegacyReplace,
        }
    }
}

#[cfg(windows)]
fn rename_information_length(name_bytes: usize) -> usize {
    std::cmp::max(
        size_of::<FILE_RENAME_INFORMATION>(),
        offset_of!(FILE_RENAME_INFORMATION, FileName) + name_bytes,
    )
}

#[cfg(windows)]
fn error_from_status(status: NTSTATUS) -> std::io::Error {
    if status == STATUS_OBJECT_NAME_COLLISION {
        return std::io::Error::from_raw_os_error(ERROR_ALREADY_EXISTS as i32);
    }
    let dos = unsafe { RtlNtStatusToDosError(status) };
    std::io::Error::from_raw_os_error(dos as i32)
}

#[cfg(windows)]
fn is_unsupported_class(status: NTSTATUS) -> bool {
    matches!(
        status,
        STATUS_INVALID_INFO_CLASS | STATUS_NOT_IMPLEMENTED | STATUS_NOT_SUPPORTED
    )
}

#[cfg(windows)]
fn rename_relative_once(
    file: &File,
    directory: &File,
    leaf: &std::ffi::OsStr,
    mode: RenameMode,
) -> std::io::Result<NTSTATUS> {
    let name: Vec<u16> = leaf.encode_wide().collect();
    if name.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "an output file name cannot contain an embedded NUL",
        ));
    }
    if name.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "an output file name cannot be empty",
        ));
    }
    let name_bytes = name.len().checked_mul(2).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "output name is too long")
    })?;
    let total = rename_information_length(name_bytes);
    let length = u32::try_from(total).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "output name is too long")
    })?;
    let name_length = u32::try_from(name_bytes).map_err(|_| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "output name is too long")
    })?;
    let mut storage = vec![0_u64; total.div_ceil(size_of::<u64>())];
    let information = storage.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
    unsafe {
        if let Some(flags) = mode.flags() {
            (*information).Anonymous.Flags = flags;
        }
        if let Some(replace) = mode.replace_if_exists() {
            (*information).Anonymous.ReplaceIfExists = replace;
        }
        (*information).RootDirectory = directory.as_raw_handle() as HANDLE;
        (*information).FileNameLength = name_length;
        let destination = (&raw mut (*information).FileName).cast::<u16>();
        std::ptr::copy_nonoverlapping(name.as_ptr(), destination, name.len());
    }
    let mut iosb = IO_STATUS_BLOCK::default();
    let status = unsafe {
        NtSetInformationFile(
            file.as_raw_handle() as HANDLE,
            &raw mut iosb,
            storage.as_ptr().cast(),
            length,
            mode.class(),
        )
    };
    Ok(status)
}

#[cfg(windows)]
fn rename_relative_with(
    file: &File,
    directory: &File,
    leaf: &std::ffi::OsStr,
    mode: RenameMode,
) -> std::io::Result<()> {
    let status = rename_relative_once(file, directory, leaf, mode)?;
    if status >= 0 {
        return Ok(());
    }
    if !is_unsupported_class(status) || mode.legacy() == mode {
        return Err(error_from_status(status));
    }
    let status = rename_relative_once(file, directory, leaf, mode.legacy())?;
    if status >= 0 {
        Ok(())
    } else {
        Err(error_from_status(status))
    }
}

#[cfg(windows)]
fn rename_relative(file: &File, directory: &File, leaf: &std::ffi::OsStr) -> std::io::Result<()> {
    rename_relative_with(file, directory, leaf, RenameMode::NoReplace)
}

#[cfg(windows)]
fn delete_by_handle(file: &File) -> std::io::Result<()> {
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    let ok = unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle() as HANDLE,
            FileDispositionInfo,
            (&raw const disposition).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn backup_leaf_name(destination: &Path) -> OsString {
    unique_leaf_name(".aitierlist-backup-", destination)
}

pub fn helper_leaf_name(destination: &Path) -> OsString {
    unique_leaf_name(".aitierlist-helper-", destination)
}

fn unique_leaf_name(prefix: &str, destination: &Path) -> OsString {
    let mut name = OsString::from(prefix);
    name.push(unique_token());
    if let Some(extension) = destination.extension() {
        name.push(".");
        name.push(extension);
    }
    name
}

fn unique_token() -> String {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = STAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{:08x}-{nonce:032x}-{sequence:016x}", std::process::id())
}

#[derive(Debug)]
pub struct OwnedStagingFile {
    parent: DirectoryLease,
    path: PathBuf,
    file: Option<File>,
    identity: FileIdentity,
    kept: bool,
}

impl OwnedStagingFile {
    pub fn create_beside(destination: &Path) -> Result<Self> {
        Self::create_beside_named(destination, stage_leaf_name(destination))
    }

    pub fn create_beside_named(destination: &Path, first: OsString) -> Result<Self> {
        let directory = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = lease_directory(directory)?;
        Self::create_in(parent, destination, first)
    }

    pub fn create_in(parent: DirectoryLease, destination: &Path, first: OsString) -> Result<Self> {
        let family: fn(&Path) -> OsString =
            if first.to_string_lossy().starts_with(".aitierlist-helper-") {
                helper_leaf_name
            } else {
                stage_leaf_name
            };
        let mut next = Some(first);
        for _ in 0..256 {
            let candidate = parent
                .resolved
                .join(next.take().unwrap_or_else(|| family(destination)));
            match create_owned_stage(&candidate) {
                Ok(file) => {
                    let (file, identity) = identify_owned_stage(file)?;
                    return Ok(OwnedStagingFile {
                        parent,
                        path: candidate,
                        file: Some(file),
                        identity,
                        kept: false,
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("creating a staging file in {}", parent.path.display())
                    });
                }
            }
        }
        bail!(
            "could not reserve a staging file beside {}",
            destination.display()
        )
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn parent(&self) -> &DirectoryLease {
        &self.parent
    }

    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub fn file(&mut self) -> Result<&mut File> {
        self.file
            .as_mut()
            .context("the staging file handle is already closed")
    }

    pub fn close_handle(&mut self) {
        self.file = None;
    }

    pub fn keep(mut self) -> PathBuf {
        self.kept = true;
        self.path.clone()
    }

    pub fn mark_kept(&mut self) {
        self.kept = true;
    }
}

impl Drop for OwnedStagingFile {
    fn drop(&mut self) {
        if self.kept {
            return;
        }
        if let Some(file) = self.file.take() {
            let _ = delete_by_handle(&file);
        }
    }
}

#[cfg(windows)]
fn open_for_disposal(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .access_mode(DELETE | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(windows)]
pub fn open_for_rename(path: &Path) -> Result<File> {
    let file =
        open_for_move(path).with_context(|| format!("opening {} to rename it", path.display()))?;
    reject_indirection(&file, path)?;
    Ok(file)
}

#[cfg(windows)]
fn open_for_move(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .access_mode(DELETE | FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

pub fn remove_recorded_object(path: &Path, expected: FileIdentity) -> Result<bool> {
    let file = match open_for_disposal(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => {
            return Err(e).with_context(|| format!("opening {} to retire it", path.display()));
        }
    };
    if identity_of(&file)? != expected {
        return Ok(false);
    }
    delete_by_handle(&file).with_context(|| format!("removing {}", path.display()))?;
    Ok(true)
}

pub fn move_recorded_object_no_replace(
    source: &Path,
    expected: FileIdentity,
    destination: &Path,
) -> Result<bool> {
    let source_directory = source
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let destination_directory = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let source_leaf = source
        .file_name()
        .with_context(|| format!("{} has no file name", source.display()))?;
    let destination_leaf = destination
        .file_name()
        .with_context(|| format!("{} has no file name", destination.display()))?;
    if source_leaf == destination_leaf && source_directory == destination_directory {
        bail!("the recorded move source and destination are the same name");
    }

    let source_parent = lease_directory(source_directory)?;
    let destination_parent = lease_directory(destination_directory)?;
    if identity_of(source_parent.file())? != identity_of(destination_parent.file())? {
        bail!("the recorded move source and destination are not in one directory");
    }
    let source_child = source_parent.resolved_path().join(source_leaf);
    let destination_child = destination_parent.resolved_path().join(destination_leaf);

    let file = match open_for_move(&source_child) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("opening {} to restore it", source.display()));
        }
    };
    reject_indirection(&file, &source_child)?;
    if identity_of(&file)? != expected {
        return Ok(false);
    }
    rename_relative(&file, destination_parent.file(), destination_leaf).with_context(|| {
        format!(
            "restoring {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    if identity_of(&file)? != expected {
        bail!("the restored object's live identity changed during its rename");
    }
    match existing_identity(&destination_child)? {
        Some(identity) if identity == expected => {}
        Some(_) => bail!("the restored destination names a different object"),
        None => bail!("the restored destination is absent after its rename"),
    }
    Ok(true)
}

pub fn replace_file_contents(destination: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;

    let directory = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let leaf = destination
        .file_name()
        .with_context(|| format!("{} has no file name", destination.display()))?
        .to_os_string();
    let parent = lease_directory(directory)?;
    let mut stage = OwnedStagingFile::create_in(parent, destination, stage_leaf_name(destination))?;

    {
        let file = stage.file()?;
        file.write_all(bytes)
            .with_context(|| format!("writing {}", destination.display()))?;
        file.flush()
            .with_context(|| format!("flushing {}", destination.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", destination.display()))?;
    }

    publish_over(&mut stage, &leaf)
        .with_context(|| format!("replacing {}", destination.display()))?;
    stage.mark_kept();
    Ok(())
}

fn publish_over(stage: &mut OwnedStagingFile, leaf: &std::ffi::OsStr) -> Result<()> {
    let parent = stage.parent().file().try_clone()?;
    let file = stage.file()?;
    replace_relative(file, &parent, leaf).map_err(Into::into)
}

#[cfg(windows)]
fn replace_relative(file: &File, directory: &File, leaf: &std::ffi::OsStr) -> std::io::Result<()> {
    rename_relative_with(file, directory, leaf, RenameMode::Replace)
}

pub fn hash_reader_with(
    reader: &mut impl std::io::Read,
    overflow: &'static str,
    mut visit: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<(u64, [u8; 32])> {
    let mut hasher = sha2::Sha256::new();
    let mut copied = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(read as u64)
            .ok_or_else(|| std::io::Error::other(overflow))?;
        sha2::Digest::update(&mut hasher, &buffer[..read]);
        visit(&buffer[..read])?;
    }
    Ok((copied, sha2::Digest::finalize(hasher).into()))
}

pub fn hash_reader(
    reader: &mut impl std::io::Read,
    overflow: &'static str,
) -> std::io::Result<(u64, [u8; 32])> {
    hash_reader_with(reader, overflow, |_| Ok(()))
}
