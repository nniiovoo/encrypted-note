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
//! reported as done, [`CommitStatus::Uncertain`] if that final flush could not be confirmed.
//! On Windows the rename is `std::fs::rename` (which copes with a target another program has
//! open), followed by a flush of the renamed file.
//!
//! Safety Copy retention: keep the 10 newest `auto-*`, and also the first `auto-*` of each day
//! for 30 days. Every save makes a copy, so by count alone eleven quick saves would push out
//! everything older; the first copy of a day is the Vault as it was before that day's changes,
//! and it stays whatever happens later that day. Keep `replaced-*` for at least 30 days (delete
//! older ones), and the newest `replaced-*` for as long as it is the newest, so one wrong clock
//! reading can't remove it. Timestamps come from the caller (`now`, Unix seconds). A new Safety
//! Copy is stamped with `max(now, newest existing created_at)` so that a clock going backwards
//! can never make the copy just taken look older than the others (and be pruned at once); the
//! same-second counter `<n>` is shared by both kinds, so "newest first" is always exact. A
//! Safety Copy never replaces another file, and none is written when the newest one is of the
//! same kind and already holds the same bytes (a save that failed after its copy was taken,
//! tried again). A copy of the other kind does not count, since it may be pruned sooner: a
//! Backup opened after a failed save still gets its own `replaced-*` copy.
//!
//! No file larger than [`MAX_FILE_BYTES`] is read or written: a real Vault is far smaller, and
//! a picked "Backup" of any size must not be pulled into memory.
//!
//! Single instance: `VaultDir::open` takes an exclusive `File::try_lock` on `.lock`; if another
//! process holds it, `StoreError::AlreadyOpen`.

#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

pub const VAULT_FILE: &str = "vault.enote";
pub const SETTINGS_FILE: &str = "settings.json";
pub const SAFETY_DIR: &str = "safety";
pub const AUTO_SAFETY_KEEP: usize = 10;
pub const REPLACED_SAFETY_KEEP_SECS: i64 = 30 * 24 * 60 * 60;
/// How long the first automatic Safety Copy of each day is kept.
pub const DAILY_SAFETY_KEEP_SECS: i64 = 30 * 24 * 60 * 60;
/// The largest Vault, Safety Copy or Backup this crate reads or writes (64 MiB).
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum StoreError {
    AlreadyOpen,
    NoVault,
    NotFound,
    /// The `verify` callback rejected what was written; nothing was replaced.
    VerifyFailed,
    /// A sandboxed file picker granted only the existing file, so a sibling temp file cannot be made.
    BackupOverwriteNotAtomic,
    /// Larger than [`MAX_FILE_BYTES`]; it was not read (or not written).
    TooLarge,
    Io(std::io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitStatus {
    Durable,
    Uncertain,
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
    ) -> Result<CommitStatus, StoreError> {
        self.install_vault(bytes, verify, now, SafetyKind::Automatic)
    }

    /// Install a Backup (or restored Safety Copy) as the Vault. The previous version becomes a
    /// `Replaced` Safety Copy.
    pub fn replace_vault(
        &self,
        bytes: &[u8],
        verify: &dyn Fn(&[u8]) -> bool,
        now: i64,
    ) -> Result<CommitStatus, StoreError> {
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
            .commit(&self.root.join(SETTINGS_FILE))?;
        Ok(())
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
    ) -> Result<CommitStatus, StoreError> {
        // Write and verify the new version first; until the final rename nothing else changes
        // except (possibly) an extra Safety Copy of the current version.
        let temp = TempFile::write_verified(&self.root, bytes, verify)?;
        let copy = match read_if_exists(&self.vault_path())? {
            Some(previous) => self.keep_safety_copy(&previous, previous_becomes, now)?,
            None => CommitStatus::Durable,
        };
        let status = temp.commit(&self.vault_path())?;
        if status == CommitStatus::Durable {
            self.apply_retention(now);
        }
        // A Safety Copy whose folder could not be flushed is no reason to refuse the save.
        Ok(if copy == CommitStatus::Uncertain {
            CommitStatus::Uncertain
        } else {
            status
        })
    }

    fn keep_safety_copy(
        &self,
        bytes: &[u8],
        kind: SafetyKind,
        now: i64,
    ) -> Result<CommitStatus, StoreError> {
        let dir = self.safety_dir();
        create_private_dir(&dir)?;
        let newest = self.numbered_safety_copies()?.into_iter().next();
        // Only a copy of the same kind makes this one unnecessary: the kinds are kept by
        // different rules, so an `auto-*` copy can't stand in for the `replaced-*` one asked for.
        if let Some((copy, _)) = &newest
            && copy.kind == kind
            && read_if_exists(&dir.join(&copy.id))?.as_deref() == Some(bytes)
        {
            return Ok(CommitStatus::Durable);
        }
        let (stamp, mut n) = match newest {
            Some((newest, n)) if newest.created_at >= now => {
                (newest.created_at, n.saturating_add(1))
            }
            _ => (now, 0),
        };
        // Our own names never collide; a file somebody else put there is not to be replaced.
        let mut path = dir.join(safety_id(kind, stamp, n));
        let mut tries = 0;
        while path.exists() {
            tries += 1;
            if tries > 1024 {
                return Err(io::Error::other("no free Safety Copy name").into());
            }
            n = n.checked_add(1).unwrap_or(0);
            path = dir.join(safety_id(kind, stamp, n));
        }
        TempFile::write_verified(&dir, bytes, &|_| true)?.commit(&path)
    }

    /// Best effort: the save itself already succeeded, so failing to prune is not an error.
    fn apply_retention(&self, now: i64) {
        let Ok(copies) = self.safety_copies() else {
            return;
        };
        let autos: Vec<&SafetyCopyInfo> = copies
            .iter()
            .filter(|c| c.kind == SafetyKind::Automatic)
            .collect();
        // Newest first, so the last copy seen for a day is that day's first.
        let mut first_of_day: std::collections::BTreeMap<i64, &str> = Default::default();
        for copy in &autos {
            first_of_day.insert(copy.created_at.div_euclid(DAY_SECS), &copy.id);
        }
        let expired_autos = autos.iter().skip(AUTO_SAFETY_KEEP).filter(|c| {
            let days_first = first_of_day.get(&c.created_at.div_euclid(DAY_SECS));
            days_first != Some(&c.id.as_str())
                || now.saturating_sub(c.created_at) > DAILY_SAFETY_KEEP_SECS
        });
        let expired_replaced = copies
            .iter()
            .filter(|c| c.kind == SafetyKind::Replaced)
            .skip(1)
            .filter(|c| now.saturating_sub(c.created_at) > REPLACED_SAFETY_KEEP_SECS);
        for copy in expired_autos.copied().chain(expired_replaced) {
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

const DAY_SECS: i64 = 24 * 60 * 60;

/// Read a Vault file the owner picked (a Backup). Refuses anything that is not an ordinary file
/// of at most [`MAX_FILE_BYTES`].
pub fn read_file(path: &Path) -> Result<Vec<u8>, StoreError> {
    read_if_exists(path)?.ok_or_else(|| io::Error::from(io::ErrorKind::NotFound).into())
}

/// `None` if `path` does not exist. The size is checked before anything is read, and the read
/// itself stops at the limit in case the file grows meanwhile.
fn read_if_exists(path: &Path) -> Result<Option<Vec<u8>>, StoreError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::other("not an ordinary file").into());
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(StoreError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(StoreError::TooLarge);
    }
    Ok(Some(bytes))
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
) -> Result<CommitStatus, StoreError> {
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    match TempFile::write_verified(parent, bytes, verify) {
        Ok(temp) => temp.commit(dest),
        // App Sandbox: a save panel grants the picked file, not its folder, so no temp file can
        // be made beside it. A new file is safe to write; overwriting in place could destroy it.
        Err(StoreError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied => {
            write_new_backup_without_folder_access(dest, bytes, verify)
        }
        Err(e) => Err(e),
    }
}

fn write_new_backup_without_folder_access(
    dest: &Path,
    bytes: &[u8],
    verify: &dyn Fn(&[u8]) -> bool,
) -> Result<CommitStatus, StoreError> {
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(StoreError::TooLarge);
    }
    let mut file = match private_options().write(true).create_new(true).open(dest) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            return Err(StoreError::BackupOverwriteNotAtomic);
        }
        result => result?,
    };
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        let read_back = fs::read(dest)?;
        if read_back != bytes || !verify(&read_back) {
            return Err(StoreError::VerifyFailed);
        }
        // The file itself is flushed and checked; only its folder entry could not be.
        Ok(CommitStatus::Uncertain)
    })();
    if result.is_err() {
        let _ = fs::remove_file(dest);
    }
    result
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
        // What can't be read back later must not be written now.
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(StoreError::TooLarge);
        }
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

    /// Atomically replace `target`; report uncertainty if the directory entry cannot be flushed.
    fn commit(mut self, target: &Path) -> Result<CommitStatus, StoreError> {
        #[cfg(unix)]
        let dir = File::open(self.path.parent().unwrap_or(Path::new(".")))?;
        #[cfg(unix)]
        return self.replace_and_flush(target, || dir.sync_all());
        #[cfg(windows)]
        {
            self.replace(target)?;
            // No directory handle to flush here; flush the renamed file, and say so if even
            // that can't be confirmed.
            let flushed = OpenOptions::new()
                .write(true)
                .open(target)
                .and_then(|file| file.sync_all());
            return Ok(if flushed.is_ok() {
                CommitStatus::Durable
            } else {
                CommitStatus::Uncertain
            });
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.replace(target)?;
            Ok(CommitStatus::Uncertain)
        }
    }

    fn replace(&mut self, target: &Path) -> Result<(), StoreError> {
        fs::rename(&self.path, target)?;
        self.committed = true;
        Ok(())
    }

    #[cfg(unix)]
    fn replace_and_flush(
        &mut self,
        target: &Path,
        flush: impl FnOnce() -> io::Result<()>,
    ) -> Result<CommitStatus, StoreError> {
        self.replace(target)?;
        Ok(if flush().is_ok() {
            CommitStatus::Durable
        } else {
            CommitStatus::Uncertain
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{CommitStatus, StoreError, TempFile, write_new_backup_without_folder_access};

    #[cfg(unix)]
    #[test]
    fn post_rename_flush_failure_reports_uncertain_without_losing_installed_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("vault.enote");
        std::fs::write(&target, b"fake old vault").unwrap();
        let mut temp = TempFile::write_verified(dir.path(), b"fake new vault", &|_| true).unwrap();
        let status = temp
            .replace_and_flush(&target, || {
                Err(std::io::Error::other("injected flush failure"))
            })
            .unwrap();
        assert_eq!(std::fs::read(target).unwrap(), b"fake new vault");
        assert_eq!(status, CommitStatus::Uncertain);
    }

    #[test]
    fn sandbox_fallback_preserves_an_existing_backup_and_removes_failed_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let existing = dir.path().join("existing.enote");
        std::fs::write(&existing, b"fake old backup").unwrap();
        assert!(matches!(
            write_new_backup_without_folder_access(&existing, b"fake new backup", &|_| true),
            Err(StoreError::BackupOverwriteNotAtomic)
        ));
        assert_eq!(std::fs::read(&existing).unwrap(), b"fake old backup");

        let new = dir.path().join("new.enote");
        assert!(matches!(
            write_new_backup_without_folder_access(&new, b"fake new backup", &|_| false),
            Err(StoreError::VerifyFailed)
        ));
        assert!(!new.exists());
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
/// starting with "OneDrive - ", "Dropbox", "Google Drive"/"My Drive", and `Library/CloudStorage/*`
/// (which on Mac also hosts OneDrive/Dropbox/Google Drive: report the specific provider from the
/// folder name when recognisable, else ICloudDrive). `.` and `..` components are resolved
/// lexically; symlinks are not followed (canonicalize first if that matters).
///
/// `home` is not trusted to be the real home folder: inside the Mac App Sandbox it is the app's
/// container, while a file panel returns real paths. So `Library/CloudStorage` is matched
/// wherever it appears, and on a Mac `/Users/<name>/Desktop` and `/Users/<name>/Documents`
/// count as well as `home`'s own.
pub fn cloud_location(path: &Path, home: &Path, platform_is_mac: bool) -> Option<CloudProvider> {
    let parts = lowercase_components(path);
    let home_parts = lowercase_components(home);
    let under_home = parts
        .strip_prefix(home_parts.as_slice())
        .filter(|_| !home_parts.is_empty());

    if let Some(at) = parts
        .windows(2)
        .position(|pair| pair[0] == "library" && pair[1] == "cloudstorage")
    {
        return Some(
            parts
                .get(at + 2)
                .map_or(CloudProvider::ICloudDrive, |f| cloud_storage_provider(f)),
        );
    }

    if let Some(provider) = parts.iter().find_map(|c| provider_folder(c)) {
        return Some(provider);
    }

    let synced = |folder: &String| folder == "desktop" || folder == "documents";
    let in_home = matches!(under_home, Some([first, ..]) if synced(first));
    let in_a_users_home = matches!(parts.as_slice(), [root, users, _, folder, ..] if root == "/" && users == "users" && synced(folder));
    (platform_is_mac && (in_home || in_a_users_home))
        .then_some(CloudProvider::MacDesktopOrDocuments)
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
