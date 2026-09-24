//! Vault store: everything about the files on disk. Knows nothing about crypto; it moves
//! opaque bytes and calls a `verify` callback the app supplies (which decrypts in memory).
//!
//! Layout inside the data folder (created 0700 on Unix; files 0600):
//! ```text
//! <root>/                      e.g. ~/Library/Application Support/<bundle-id>/Vault.noindex
//!   vault.enote                the Vault
//!   .lock                      single-instance lock file (OS advisory lock held while open)
//!   settings.json              non-secret preferences (opaque bytes to this crate)
//!   safety/auto-<unix>-<n>.enote      automatic Safety Copies (previous version on each save)
//!   safety/replaced-<unix>-<n>.enote  Safety Copies made when a Backup replaced the Vault
//!   .tmp-<random>              in-flight writes (removed at open if left over)
//! ```
//!
//! Crash-safe write (used for the Vault, settings and Backups): create a temp file in the SAME
//! directory with create_new (0600), write, `sync_all` (F_FULLFSYNC on Apple), read it back and
//! call `verify` on what was read (the app decrypts it), then rename over the target, then fsync
//! the directory (Unix). On any failure the old file is untouched and the temp file is removed.
//! The directory is opened for the fsync BEFORE the rename, so a folder that cannot be flushed
//! fails while the old file is still in place; once the rename has happened the write is
//! reported as a success (the final directory fsync is best effort).
//!
//! Safety Copy retention: keep the 10 newest `auto-*`; keep `replaced-*` for at least 30 days
//! (delete older ones). Timestamps come from the caller (`now`, Unix seconds). A new Safety Copy
//! is stamped with `max(now, newest existing created_at)` so that a clock going backwards can
//! never make the copy just taken look older than the others (and be pruned at once); the
//! same-second counter `<n>` is shared by both kinds, so "newest first" is always exact.
//!
//! Single instance: `VaultDir::open` takes an exclusive `File::try_lock` on `.lock`; if another
//! process holds it, `StoreError::AlreadyOpen`.

#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

pub const VAULT_FILE: &str = "vault.enote";
pub const SETTINGS_FILE: &str = "settings.json";
pub const SAFETY_DIR: &str = "safety";
pub const AUTO_SAFETY_KEEP: usize = 10;
pub const REPLACED_SAFETY_KEEP_SECS: i64 = 30 * 24 * 60 * 60;

#[derive(Debug)]
pub enum StoreError {
    AlreadyOpen,
    NoVault,
    NotFound,
    /// The `verify` callback rejected what was written; nothing was replaced.
    VerifyFailed,
    Io(std::io::Error),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for StoreError {}
impl From<std::io::Error> for StoreError {
    fn from(e: std::io::Error) -> Self {
        StoreError::Io(e)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafetyKind {
    Automatic,
    Replaced,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafetyCopyInfo {
    /// File name inside `safety/`; pass back to `read_safety_copy`.
    pub id: String,
    pub kind: SafetyKind,
    /// Unix seconds when it became a Safety Copy.
    pub created_at: i64,
}

/// An open data folder, holding the single-instance lock until dropped.
pub struct VaultDir {
    root: PathBuf,
    /// Held (and locked) for as long as this `VaultDir` lives; the OS releases the lock on close.
    _lock: File,
}

const LOCK_FILE: &str = ".lock";
const TEMP_PREFIX: &str = ".tmp-";
const SAFETY_EXT: &str = ".enote";

impl VaultDir {
    /// Create the folder if needed (0700), take the lock, remove leftover temp files.
    pub fn open(root: &Path) -> Result<VaultDir, StoreError> {
        create_private_dir(root)?;
        let lock = private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join(LOCK_FILE))?;
        lock.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => StoreError::AlreadyOpen,
            fs::TryLockError::Error(e) => StoreError::Io(e),
        })?;
        remove_leftover_temp_files(root)?;
        let safety = root.join(SAFETY_DIR);
        if safety.is_dir() {
            remove_leftover_temp_files(&safety)?;
        }
        Ok(VaultDir {
            root: root.to_path_buf(),
            _lock: lock,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn has_vault(&self) -> bool {
        self.vault_path().is_file()
    }

    pub fn read_vault(&self) -> Result<Vec<u8>, StoreError> {
        read_if_exists(&self.vault_path())?.ok_or(StoreError::NoVault)
    }

    /// Crash-safe write of a new Vault version. The previous version (if any) becomes an
    /// automatic Safety Copy; retention is applied.
    pub fn save_vault(
        &self,
        bytes: &[u8],
        verify: &dyn Fn(&[u8]) -> bool,
        now: i64,
    ) -> Result<(), StoreError> {
        self.install_vault(bytes, verify, now, SafetyKind::Automatic)
    }

    /// Install a Backup (or restored Safety Copy) as the Vault. The previous version becomes a
    /// `Replaced` Safety Copy.
    pub fn replace_vault(
        &self,
        bytes: &[u8],
        verify: &dyn Fn(&[u8]) -> bool,
        now: i64,
    ) -> Result<(), StoreError> {
        self.install_vault(bytes, verify, now, SafetyKind::Replaced)
    }

    /// Newest first.
    pub fn safety_copies(&self) -> Result<Vec<SafetyCopyInfo>, StoreError> {
        Ok(self
            .numbered_safety_copies()?
            .into_iter()
            .map(|(c, _)| c)
            .collect())
    }

    pub fn read_safety_copy(&self, id: &str) -> Result<Vec<u8>, StoreError> {
        parse_safety_id(id).ok_or(StoreError::NotFound)?;
        read_if_exists(&self.safety_dir().join(id))?.ok_or(StoreError::NotFound)
    }

    pub fn read_settings(&self) -> Result<Option<Vec<u8>>, StoreError> {
        read_if_exists(&self.root.join(SETTINGS_FILE))
    }

    pub fn write_settings(&self, bytes: &[u8]) -> Result<(), StoreError> {
        TempFile::write_verified(&self.root, bytes, &|_| true)?
            .commit(&self.root.join(SETTINGS_FILE))
    }

    fn vault_path(&self) -> PathBuf {
        self.root.join(VAULT_FILE)
    }

    fn safety_dir(&self) -> PathBuf {
        self.root.join(SAFETY_DIR)
    }

    fn install_vault(
        &self,
        bytes: &[u8],
        verify: &dyn Fn(&[u8]) -> bool,
        now: i64,
        previous_becomes: SafetyKind,
    ) -> Result<(), StoreError> {
        // Write and verify the new version first; until the final rename nothing else changes
        // except (possibly) an extra Safety Copy of the current version.
        let temp = TempFile::write_verified(&self.root, bytes, verify)?;
        if let Some(previous) = read_if_exists(&self.vault_path())? {
            self.keep_safety_copy(&previous, previous_becomes, now)?;
        }
        temp.commit(&self.vault_path())?;
        self.apply_retention(now);
        Ok(())
    }

    fn keep_safety_copy(&self, bytes: &[u8], kind: SafetyKind, now: i64) -> Result<(), StoreError> {
        let dir = self.safety_dir();
        create_private_dir(&dir)?;
        let (stamp, n) = match self.numbered_safety_copies()?.first() {
            Some((newest, n)) if newest.created_at >= now => {
                (newest.created_at, n.saturating_add(1))
            }
            _ => (now, 0),
        };
        TempFile::write_verified(&dir, bytes, &|_| true)?
            .commit(&dir.join(safety_id(kind, stamp, n)))
    }

    /// Best effort: the save itself already succeeded, so failing to prune is not an error.
    fn apply_retention(&self, now: i64) {
        let Ok(copies) = self.safety_copies() else {
            return;
        };
        let expired_autos = copies
            .iter()
            .filter(|c| c.kind == SafetyKind::Automatic)
            .skip(AUTO_SAFETY_KEEP);
        let expired_replaced = copies.iter().filter(|c| {
            c.kind == SafetyKind::Replaced
                && now.saturating_sub(c.created_at) > REPLACED_SAFETY_KEEP_SECS
        });
        for copy in expired_autos.chain(expired_replaced) {
            let _ = fs::remove_file(self.safety_dir().join(&copy.id));
        }
    }

    /// Every recognised Safety Copy with its same-second counter `<n>`, newest first.
    fn numbered_safety_copies(&self) -> Result<Vec<(SafetyCopyInfo, u64)>, StoreError> {
        let entries = match fs::read_dir(self.safety_dir()) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut copies = Vec::new();
        for entry in entries {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Some((kind, created_at, n)) = parse_safety_id(&name) else {
                continue;
            };
            if entry.file_type()?.is_file() {
                copies.push((
                    SafetyCopyInfo {
                        id: name,
                        kind,
                        created_at,
                    },
                    n,
                ));
            }
        }
        copies.sort_by_key(|(c, n)| std::cmp::Reverse((c.created_at, *n)));
        Ok(copies)
    }
}

/// `None` if `path` does not exist.
fn read_if_exists(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn safety_prefix(kind: SafetyKind) -> &'static str {
    match kind {
        SafetyKind::Automatic => "auto-",
        SafetyKind::Replaced => "replaced-",
    }
}

fn safety_id(kind: SafetyKind, created_at: i64, n: u64) -> String {
    format!("{}{created_at}-{n}{SAFETY_EXT}", safety_prefix(kind))
}

/// Accepts only names this crate itself generates (which also rules out any path tricks).
fn parse_safety_id(id: &str) -> Option<(SafetyKind, i64, u64)> {
    for kind in [SafetyKind::Automatic, SafetyKind::Replaced] {
        let Some(rest) = id.strip_prefix(safety_prefix(kind)) else {
            continue;
        };
        let (ts, n) = rest.strip_suffix(SAFETY_EXT)?.rsplit_once('-')?;
        let (created_at, n) = (ts.parse::<i64>().ok()?, n.parse::<u64>().ok()?);
        return (safety_id(kind, created_at, n) == id).then_some((kind, created_at, n));
    }
    None
}

/// Crash-safe write of a Backup to `dest` (a full file path chosen by the owner), verified by
/// reading it back. Overwrites an existing file at `dest` only after verification succeeds.
pub fn write_backup(
    dest: &Path,
    bytes: &[u8],
    verify: &dyn Fn(&[u8]) -> bool,
) -> Result<(), StoreError> {
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    TempFile::write_verified(parent, bytes, verify)?.commit(dest)
}

/// A written-and-verified temp file; removed on drop unless committed.
struct TempFile {
    path: PathBuf,
    committed: bool,
}

impl TempFile {
    /// Create a temp file in `dir` (create_new, 0600), write, flush to disk, read it back and
    /// check it is exactly `bytes` and that `verify` accepts it.
    fn write_verified(
        dir: &Path,
        bytes: &[u8],
        verify: &dyn Fn(&[u8]) -> bool,
    ) -> Result<TempFile, StoreError> {
        let mut attempts = 0;
        let (mut file, path) = loop {
            let random = getrandom::u64().map_err(|e| io::Error::other(e.to_string()))?;
            let path = dir.join(format!("{TEMP_PREFIX}{random:016x}"));
            match private_options().write(true).create_new(true).open(&path) {
                Ok(file) => break (file, path),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists && attempts < 8 => attempts += 1,
                Err(e) => return Err(e.into()),
            }
        };
        let temp = TempFile {
            path,
            committed: false,
        };
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let read_back = fs::read(&temp.path)?;
        if read_back != bytes || !verify(&read_back) {
            return Err(StoreError::VerifyFailed);
        }
        Ok(temp)
    }

    /// Atomically rename over `target`. The directory is opened for the fsync first, so it can
    /// fail while the old file is intact; once renamed, the directory fsync is best effort.
    /// (Windows has no directory fsync; the rename is flushed with the file system journal.)
    fn commit(mut self, target: &Path) -> Result<(), StoreError> {
        #[cfg(unix)]
        let dir = File::open(self.path.parent().unwrap_or(Path::new(".")))?;
        fs::rename(&self.path, target)?;
        self.committed = true;
        #[cfg(unix)]
        let _ = dir.sync_all();
        Ok(())
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn remove_leftover_temp_files(dir: &Path) -> Result<(), StoreError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let is_temp = entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(TEMP_PREFIX));
        if is_temp && entry.file_type()?.is_file() {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

fn private_options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut options = OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn create_private_dir(dir: &Path) -> Result<(), StoreError> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudProvider {
    /// iCloud Drive (`~/Library/Mobile Documents`) or a File Provider folder under `~/Library/CloudStorage`.
    ICloudDrive,
    /// `~/Desktop` or `~/Documents` on a Mac: synced if "Desktop & Documents Folders" is on.
    MacDesktopOrDocuments,
    OneDrive,
    Dropbox,
    GoogleDrive,
}

/// Is `path` (a folder or file the owner picked) inside a cloud-synced location? `home` is the
/// owner's home directory; `platform_is_mac` enables the Desktop/Documents rule. Pure path logic.
/// Matching is on path components (case-insensitive), e.g. any component named "OneDrive" or
/// starting with "OneDrive - ", "Dropbox", "Google Drive"/"My Drive", and `~/Library/CloudStorage/*`
/// (which on Mac also hosts OneDrive/Dropbox/Google Drive: report the specific provider from the
/// folder name when recognisable, else ICloudDrive). `.` and `..` components are resolved
/// lexically; symlinks are not followed (canonicalize first if that matters).
pub fn cloud_location(path: &Path, home: &Path, platform_is_mac: bool) -> Option<CloudProvider> {
    let parts = lowercase_components(path);
    let home_parts = lowercase_components(home);
    let under_home = parts
        .strip_prefix(home_parts.as_slice())
        .filter(|_| !home_parts.is_empty());

    match under_home {
        Some([library, storage, rest @ ..])
            if library == "library" && storage == "cloudstorage" =>
        {
            return Some(
                rest.first()
                    .map_or(CloudProvider::ICloudDrive, |f| cloud_storage_provider(f)),
            );
        }
        Some([library, mobile, ..]) if library == "library" && mobile == "mobile documents" => {
            return Some(CloudProvider::ICloudDrive);
        }
        _ => {}
    }

    if let Some(provider) = parts.iter().find_map(|c| provider_folder(c)) {
        return Some(provider);
    }

    if platform_is_mac
        && let Some([first, ..]) = under_home
        && (first == "desktop" || first == "documents")
    {
        return Some(CloudProvider::MacDesktopOrDocuments);
    }
    None
}

/// Lowercased components with `.` dropped and `..` resolved lexically (no file system access),
/// so e.g. `~/x/../Documents` matches like `~/Documents`. `..` at the root stays at the root.
fn lowercase_components(path: &Path) -> Vec<String> {
    let mut parts: Vec<Component<'_>> = Vec::new();
    for c in path.components() {
        match (c, parts.last()) {
            (Component::CurDir, _)
            | (Component::ParentDir, Some(Component::RootDir | Component::Prefix(_))) => {}
            (Component::ParentDir, Some(Component::Normal(_))) => {
                parts.pop();
            }
            _ => parts.push(c),
        }
    }
    parts
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

/// A folder under `~/Library/CloudStorage`, e.g. "OneDrive-Personal", "GoogleDrive-<account>".
fn cloud_storage_provider(folder: &str) -> CloudProvider {
    if folder.starts_with("onedrive") {
        CloudProvider::OneDrive
    } else if folder.starts_with("dropbox") {
        CloudProvider::Dropbox
    } else if folder.starts_with("googledrive") || folder.starts_with("google drive") {
        CloudProvider::GoogleDrive
    } else {
        CloudProvider::ICloudDrive
    }
}

/// A well-known synced folder name found anywhere in the path (already lowercased).
fn provider_folder(component: &str) -> Option<CloudProvider> {
    match component {
        "mobile documents" | "iclouddrive" | "icloud drive" => Some(CloudProvider::ICloudDrive),
        "onedrive" => Some(CloudProvider::OneDrive),
        c if c.starts_with("onedrive - ") => Some(CloudProvider::OneDrive),
        "dropbox" => Some(CloudProvider::Dropbox),
        c if c.starts_with("dropbox (") => Some(CloudProvider::Dropbox),
        "google drive" | "my drive" => Some(CloudProvider::GoogleDrive),
        _ => None,
    }
}
