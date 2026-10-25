//! `AppCore`: everything the app bridge does, as plain Rust with no Tauri types, so it can be
//! tested with a temp folder, a fake clock and a fake clipboard.
//!
//! One method per command in `src/api.ts`, with the same name and meaning. Every method checks
//! the phase first. Errors are [`ApiError`]s whose `message` is calm, owner-facing English.
//!
//! What lives here (and nowhere else): the open Vault (Vault Key + decrypted document), the
//! session rules, the Sharing Guard state and the non-secret settings. The Wallet Key is never
//! kept: each Wallet Kind action derives it, uses it and drops it (ADR-0004). No password or
//! Recovery Key is kept either, except the Recovery Key that was just used (or just made) while
//! the owner must still set a new password (or confirm their Recovery Kit).
//!
//! Nothing is written to disk before the owner confirms their Recovery Kit. After that, every
//! change is saved at once: seal, write to a temp file, read it back, [`vault_format::reopen`]
//! it and parse the document, then rename over the Vault (store crate).

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use notes::keys::Chain;
use notes::seed::SeedCheck;
use notes::{
    Document, Filter, Kind, NewNote, NoteEdit, NoteSummary, NoteView, NotesError, WalletCipher,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use session::{Effect, ExtendRefused, LockSettings, OsEvent, RevealMode, RevealRefused, Session};
use store::{CloudProvider, SafetyKind, StoreError, VaultDir};
use vault_format::{Credential, KdfParams, Limits, RecoveryKey, UnlockedVault, WalletKey};
use zeroize::Zeroizing;

pub mod native;
mod quick_unlock;
pub use quick_unlock::{
    QuickUnlockCompletion, QuickUnlockEnrollment, QuickUnlockEnrollmentCompletion, QuickUnlockJob,
    QuickUnlockStatus,
};

// ---------- injected dependencies ----------

/// Time, injected so tests can move it.
pub trait Clock: Send {
    /// Monotonic milliseconds (session timers).
    fn now_ms(&self) -> u64;
    /// Wall-clock Unix seconds (timestamps stored in the Vault and settings).
    fn unix_secs(&self) -> i64;
    /// Clock that keeps counting through sleep; missing support disables Quick Unlock.
    fn continuous_ms(&self) -> Option<u64> {
        None
    }
}

/// The system clipboard. Implementations never read what's on it.
pub trait Clipboard: Send {
    /// Put `text` on the clipboard privately; return the clipboard's change count after the write.
    fn write(&mut self, text: &str) -> Result<u64, ApiError>;
    /// Clear it if unchanged. Returns false only when clearing should be retried.
    fn clear_if(&mut self, change_count: u64) -> bool;
}

pub struct Config {
    /// The data folder (`.../Vault.noindex`).
    pub data_dir: PathBuf,
    /// The owner's home folder, for spotting cloud-synced Backup destinations.
    pub home_dir: PathBuf,
    pub limits: Limits,
    pub clock: Box<dyn Clock>,
    pub clipboard: Box<dyn Clipboard>,
    /// Whether the app can ask the OS when hardware input was last seen anywhere on this
    /// computer (`platform::ms_since_last_input`). Where it can't, "count activity anywhere"
    /// stays off whatever the settings say.
    pub activity_anywhere_supported: bool,
}

// ---------- API shapes (src/api.ts) ----------

/// `{ code, message }`; `code` is one of the `ApiError` codes in api.ts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ApiError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialInput {
    MasterPassword(Zeroizing<String>),
    RecoveryKey(Zeroizing<String>),
}

#[derive(Deserialize)]
pub struct NoteInput {
    pub kind: Kind,
    pub title: String,
    #[serde(default)]
    pub visible: BTreeMap<String, String>,
    #[serde(default)]
    pub hidden: BTreeMap<String, Zeroizing<String>>,
}

/// `NoteInput` without `kind`. Hidden Fields: omitted = keep, "" = clear.
#[derive(Deserialize)]
pub struct NoteEditInput {
    pub title: String,
    #[serde(default)]
    pub visible: BTreeMap<String, String>,
    #[serde(default)]
    pub hidden: BTreeMap<String, Zeroizing<String>>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShowMode {
    Held,
    Accessible,
}

/// Omitted preferences keep their values. Authentication is used only for loosening Auto-lock.
/// No Debug: this input can contain the Master Password.
#[derive(Deserialize)]
pub struct SettingsInput {
    #[serde(default)]
    pub master_password: Option<Zeroizing<String>>,
    /// Non-secret stale-request guard; never substitutes for the password.
    #[serde(default)]
    pub session_generation: Option<u64>,
    pub idle_minutes: u32,
    pub lock_on_app_switch: bool,
    /// 30 or 60; anything else becomes 30.
    #[serde(default)]
    pub clipboard_clear_seconds: Option<u32>,
    #[serde(default)]
    pub count_activity_anywhere: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppStatus {
    /// Identifies this unlock session, so a pre-lock settings prompt cannot authorize a later one.
    pub session_generation: u64,
    /// Increases for every snapshot or event sent to the WebView, so older async results can be ignored.
    pub revision: u64,
    pub phase: &'static str,
    pub platform: &'static str,
    pub capture_hiding_active: Option<bool>,
    /// OS events whose lock notifications could not be installed on Windows.
    pub lock_events_unavailable: Vec<&'static str>,
    pub sharing_guard: GuardView,
    pub lock_in_ms: Option<u64>,
    pub clipboard_clear_in_ms: Option<u64>,
    /// How many copies this run of the app has put on the clipboard: a count only, never what
    /// was copied. The screens tell a new copy by it; the time left can't say (a Wallet Kind
    /// copy may have less than the copy before it, and a clear that is tried again has more).
    pub copies: u64,
    /// Visible metadata of the last successful copy; never its value or an environment assignment.
    pub clipboard_source: Option<ClipboardSource>,
    pub reveal: Option<RevealView>,
    pub failed_attempts: u32,
    pub retry_in_ms: Option<u64>,
    pub show_recovery_hint: bool,
    pub backup: BackupView,
    pub purge_trash_failed: bool,
    pub save_durability_uncertain: bool,
    /// The settings file was there but unreadable, so the strictest lock rules are in force.
    pub settings_unreadable: bool,
    pub settings: SettingsView,
    pub quick_unlock: QuickUnlockStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ClipboardSource {
    pub title: String,
    pub kind: Kind,
    pub field: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GuardView {
    pub up: bool,
    pub apps: Vec<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RevealView {
    pub note_id: String,
    pub field: String,
    pub expires_in_ms: Option<u64>,
    /// Whether `extend_reveal` would give this value more time right now.
    pub can_extend: bool,
    /// For a Wallet Kind value shown for 20 s: how many more times it can be given more time.
    /// `None` where there is no such count (an ordinary Kind, or a value shown while held).
    pub extensions_left: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BackupView {
    pub last_backup_at: Option<i64>,
    pub reminder_due: bool,
    pub unbacked_changes: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingsView {
    pub idle_minutes: u32,
    pub lock_on_app_switch: bool,
    pub seed_explainer_seen: bool,
    /// 30 or 60. A copied Wallet Kind value is cleared after 30 s either way.
    pub clipboard_clear_seconds: u32,
    pub count_activity_anywhere: bool,
    /// False where the app can't see input outside its own window: the setting then stays off.
    pub count_activity_anywhere_supported: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AssessmentView {
    pub acceptable: bool,
    pub score: u8,
    pub feedback: Vec<String>,
}

/// `create_vault` and `change_password` results. No `Debug`: it holds the Recovery Key.
#[derive(Serialize)]
pub struct RecoveryKeyShown {
    /// Shown to the owner once; `None` after a password change without Key Rotation.
    pub recovery_key: Option<Zeroizing<String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PickedFile {
    pub display_path: String,
    pub cloud: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BackupWritten {
    pub display_path: String,
    pub note_count: usize,
    /// False when the file is written and checked but its folder entry could not be flushed
    /// (a sandboxed save): the drive should be ejected properly before it is unplugged.
    pub durable: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VaultPreview {
    pub note_count: usize,
    pub changed_at: i64,
    /// `Some(true)`: this computer's Vault may hold changes this copy lacks. `None`: there is
    /// nothing to compare with, or (`replaces_vault`) the Vault here couldn't be compared.
    pub older_than_current: Option<bool>,
    /// Installing this copy replaces a Vault that exists on this computer.
    pub replaces_vault: bool,
    // The next three say what this computer's Vault holds, so the owner can be told what the
    // copy would replace. All three are `Some` only when the Vault here is Unlocked and the
    // copy is a copy of that same Vault. Otherwise all three are `None`: there is nothing to
    // compare, or the Vault here is Locked.
    /// Notes in this computer's Vault that are not in Trash.
    pub current_note_count: Option<usize>,
    /// When this computer's Vault was last changed.
    pub current_changed_at: Option<i64>,
    /// How many Notes here (not in Trash) the copy doesn't hold at all, its Trash included.
    /// A number only, never titles.
    pub only_here: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SafetyCopy {
    pub id: String,
    pub kind: &'static str,
    pub created_at: i64,
}

// ---------- state ----------

/// A decrypted Vault: the Vault Key (inside `vault`) and the document.
struct Open {
    vault: UnlockedVault,
    doc: Document,
}

enum Phase {
    NoVault,
    /// A new Vault (or a Key Rotation) waiting for the owner to confirm the Recovery Kit.
    /// Nothing is on disk yet; locking throws it away.
    ConfirmRecoveryKit {
        open: Box<Open>,
        recovery_key: RecoveryKey,
        rotation: bool,
    },
    Locked,
    /// Opened with this Recovery Key; a new Master Password must be set before anything else.
    NeedsNewPassword {
        open: Box<Open>,
        recovery_key: RecoveryKey,
    },
    Unlocked(Box<Open>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Backup,
    SafetyCopy,
}

/// A Backup or Safety Copy decrypted by `inspect_*`, waiting for "Replace". Dropped when the
/// owner closes the preview, after the idle time (two minutes while no Vault is Unlocked), on
/// anything that locks, on any phase change and on any edit.
struct Inspected {
    source: Source,
    bytes: Vec<u8>,
    open: Box<Open>,
    recovery_key: Option<RecoveryKey>,
    expires_at: u64,
}

/// The non-secret settings file.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
struct StoredSettings {
    idle_minutes: u32,
    lock_on_app_switch: bool,
    seed_explainer_seen: bool,
    last_backup_at: Option<i64>,
    oldest_unbacked_change_at: Option<i64>,
    /// Consecutive wrong credentials, so that restarting the app doesn't reset the delays.
    failed_attempts: u32,
    /// When an unlock first found Notes whose 30 days in Trash were over (see `purge_trash`).
    trash_expiry_seen_at: Option<i64>,
    /// How long a copy stays on the clipboard: 30 or 60 (the session crate keeps it to those).
    clipboard_clear_seconds: u32,
    /// The owner's choice to count hardware input anywhere on the computer as activity.
    count_activity_anywhere: bool,
    /// Header fingerprint only; authorization deadlines never persist.
    quick_unlock_binding: Option<[u8; 32]>,
}

impl Default for StoredSettings {
    fn default() -> Self {
        StoredSettings {
            idle_minutes: session::DEFAULT_IDLE_MINUTES,
            lock_on_app_switch: false,
            seed_explainer_seen: false,
            last_backup_at: None,
            oldest_unbacked_change_at: None,
            failed_attempts: 0,
            trash_expiry_seen_at: None,
            clipboard_clear_seconds: session::DEFAULT_CLIPBOARD_CLEAR_SECONDS,
            count_activity_anywhere: false,
            quick_unlock_binding: None,
        }
    }
}

impl StoredSettings {
    /// For a settings file that is there but can't be read: the strictest lock rules, not the
    /// defaults, so damaging the file can only make the app lock sooner (and clear a copy
    /// sooner, and count nothing but activity inside its own window).
    fn strictest() -> Self {
        StoredSettings {
            idle_minutes: session::IDLE_MINUTE_CHOICES[0],
            lock_on_app_switch: true,
            clipboard_clear_seconds: session::CLIPBOARD_CLEAR_SECOND_CHOICES[0],
            count_activity_anywhere: false,
            ..StoredSettings::default()
        }
    }
}

/// Where the next Backup goes: the file the owner just picked in the save dialog. Used once.
struct BackupDestination {
    path: PathBuf,
    expires_at: u64,
}

/// A picked Backup destination is forgotten if no Backup is written to it within this long.
const BACKUP_DESTINATION_MS: u64 = 10 * 60_000;
/// How long a preview of a Backup or Safety Copy is kept while no Vault is Unlocked.
const LOCKED_PREVIEW_MS: u64 = 2 * 60_000;
/// Trash is emptied of expired Notes only when two unlocks at least this far apart both find
/// them expired, so one unlock under a wrong clock removes nothing.
const PURGE_CONFIRM_SECS: i64 = 24 * 60 * 60;

/// A credential as typed, parsed.
enum Cred {
    Password(SecretString),
    Key(RecoveryKey),
}

impl Cred {
    fn credential(&self) -> Credential<'_> {
        match self {
            Cred::Password(p) => Credential::MasterPassword(p),
            Cred::Key(k) => Credential::RecoveryKey(k),
        }
    }
}

/// Seals and opens Wallet Kind Hidden Fields with a Wallet Key that lives only for one call.
struct Wallet {
    key: WalletKey,
    vault_id: [u8; 16],
}

impl WalletCipher for Wallet {
    fn seal(&self, note_id: &str, field: &str, plaintext: &[u8]) -> Result<Vec<u8>, NotesError> {
        vault_format::seal_wallet_field(&self.key, &self.vault_id, note_id, field, plaintext)
            .map_err(|_| NotesError::WalletCipher)
    }
    fn open(
        &self,
        note_id: &str,
        field: &str,
        sealed: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, NotesError> {
        vault_format::open_wallet_field(&self.key, &self.vault_id, note_id, field, sealed)
            .map_err(|_| NotesError::WalletCipher)
    }
}

const PLATFORM: &str = if cfg!(target_os = "macos") {
    "mac"
} else if cfg!(windows) {
    "windows"
} else {
    "other"
};

const GUARD_PLATFORM: guard::Platform = if cfg!(windows) {
    guard::Platform::Windows
} else {
    guard::Platform::Mac
};

pub struct AppCore {
    dir: VaultDir,
    home_dir: PathBuf,
    limits: Limits,
    clock: Box<dyn Clock>,
    clipboard: Box<dyn Clipboard>,
    /// Copies written to the clipboard since the app started (`AppStatus::copies`).
    copies: u64,
    clipboard_source: Option<ClipboardSource>,
    session: Session,
    session_generation: u64,
    guard: guard::GuardState,
    detected: Vec<&'static str>,
    capture_hiding_active: Option<bool>,
    lock_events_unavailable: Vec<&'static str>,
    phase: Phase,
    settings: StoredSettings,
    settings_unreadable: bool,
    activity_anywhere_supported: bool,
    backup_destination: Option<BackupDestination>,
    backup_to_open: Option<PathBuf>,
    inspected: Option<Inspected>,
    purge_trash_failed: bool,
    save_durability_uncertain: bool,
    native_pending: Option<native::Pending>,
    native_sequence: u64,
    native_epoch: u64,
    /// Only populated during one checked native completion; never an authentication cache.
    native_wallet: Option<Wallet>,
    quick_unlock: crate::quick_unlock::QuickUnlockGate,
    quick_unlock_available: bool,
    quick_unlock_reason: Option<&'static str>,
    quick_pending: Option<u64>,
    quick_cleanup_needed: bool,
}

impl AppCore {
    /// Opens the data folder (taking the single-instance lock) and reads the settings.
    pub fn new(config: Config) -> Result<AppCore, ApiError> {
        let dir = VaultDir::open(&config.data_dir).map_err(store_error)?;
        // No settings file yet means defaults. One that is there but can't be read means the
        // strictest rules, and the owner is told (`settings_unreadable`).
        let stored = match dir.read_settings() {
            Ok(None) => Some(StoredSettings::default()),
            Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
            Err(_) => None,
        };
        let settings_unreadable = stored.is_none();
        let settings = stored.unwrap_or_else(StoredSettings::strictest);
        let activity_anywhere_supported = config.activity_anywhere_supported;
        let mut session = Session::new(LockSettings {
            idle_minutes: settings.idle_minutes,
            lock_on_app_switch: settings.lock_on_app_switch,
            clipboard_clear_seconds: settings.clipboard_clear_seconds,
            count_activity_anywhere: settings.count_activity_anywhere
                && activity_anywhere_supported,
        });
        if settings.failed_attempts > 0 {
            session.restore_failures(settings.failed_attempts, config.clock.now_ms());
        }
        let phase = if dir.has_vault() {
            Phase::Locked
        } else {
            Phase::NoVault
        };
        let mut core = AppCore {
            dir,
            home_dir: config.home_dir,
            limits: config.limits,
            clock: config.clock,
            clipboard: config.clipboard,
            copies: 0,
            clipboard_source: None,
            session,
            session_generation: 0,
            guard: guard::GuardState::new(),
            detected: Vec::new(),
            capture_hiding_active: None,
            lock_events_unavailable: Vec::new(),
            phase,
            settings,
            settings_unreadable,
            activity_anywhere_supported,
            backup_destination: None,
            backup_to_open: None,
            inspected: None,
            purge_trash_failed: false,
            save_durability_uncertain: false,
            native_pending: None,
            native_sequence: 0,
            native_epoch: 0,
            native_wallet: None,
            quick_unlock: crate::quick_unlock::QuickUnlockGate::default(),
            quick_unlock_available: false,
            quick_unlock_reason: None,
            quick_pending: None,
            quick_cleanup_needed: false,
        };
        core.load_quick_unlock_registration();
        Ok(core)
    }

    // ---------- status & events ----------

    pub fn app_status(&self) -> AppStatus {
        let now = self.clock.now_ms();
        let shield = self.guard.shield_apps();
        let lock = self.session.settings();
        AppStatus {
            session_generation: self.session_generation,
            revision: 0,
            phase: match self.phase {
                Phase::NoVault => "no_vault",
                Phase::ConfirmRecoveryKit { .. } => "confirm_recovery_kit",
                Phase::Locked => "locked",
                Phase::NeedsNewPassword { .. } => "needs_new_password",
                Phase::Unlocked(_) => "unlocked",
            },
            platform: PLATFORM,
            capture_hiding_active: self.capture_hiding_active,
            lock_events_unavailable: self.lock_events_unavailable.clone(),
            sharing_guard: GuardView {
                up: !shield.is_empty(),
                apps: shield.into_iter().collect(),
            },
            lock_in_ms: self
                .session
                .lock_deadline()
                .map(|at| at.saturating_sub(now)),
            clipboard_clear_in_ms: self
                .session
                .clipboard_clear_at()
                .map(|at| at.saturating_sub(now)),
            copies: self.copies,
            clipboard_source: self.clipboard_source.clone(),
            reveal: self.session.current_reveal().map(|r| RevealView {
                note_id: r.note_id.clone(),
                field: r.field.clone(),
                expires_in_ms: r.expires_at.map(|at| at.saturating_sub(now)),
                // Not for one whose time is up although the tick that hides it hasn't run.
                can_extend: r.can_extend() && r.expires_at.is_some_and(|at| now < at),
                extensions_left: r.extensions_left(),
            }),
            failed_attempts: self.session.failed_attempts(),
            retry_in_ms: self.session.attempt_allowed(now).err(),
            show_recovery_hint: self.session.show_recovery_hint(),
            backup: BackupView {
                last_backup_at: self.settings.last_backup_at,
                reminder_due: session::backup_reminder_due(
                    self.settings.oldest_unbacked_change_at.map(secs_to_ms),
                    secs_to_ms(self.clock.unix_secs()),
                ),
                unbacked_changes: self.settings.oldest_unbacked_change_at.is_some(),
            },
            purge_trash_failed: self.purge_trash_failed,
            save_durability_uncertain: self.save_durability_uncertain,
            settings_unreadable: self.settings_unreadable,
            quick_unlock: self.quick_unlock_status(),
            settings: SettingsView {
                idle_minutes: lock.idle_minutes,
                lock_on_app_switch: lock.lock_on_app_switch,
                seed_explainer_seen: self.settings.seed_explainer_seen,
                clipboard_clear_seconds: lock.clipboard_clear_seconds,
                count_activity_anywhere: lock.count_activity_anywhere,
                count_activity_anywhere_supported: self.activity_anywhere_supported,
            },
        }
    }

    /// Wall-clock Unix seconds from the injected clock (e.g. for a Backup's default file name).
    pub fn unix_secs(&self) -> i64 {
        self.clock.unix_secs()
    }

    /// The owner's home folder (where the Backup panels start when no drive is plugged in).
    pub fn home_dir(&self) -> &Path {
        &self.home_dir
    }

    /// Owner activity (keeps Auto-lock away).
    pub fn activity(&mut self) {
        self.session.activity(self.clock.now_ms());
    }

    /// Whether the app should be asking the OS about input outside its window at all: the owner
    /// turned "count activity anywhere" on, this computer supports it, and a Vault is open.
    pub fn counts_activity_anywhere(&self) -> bool {
        self.session.settings().count_activity_anywhere && self.session.is_unlocked()
    }

    /// The OS saw hardware input somewhere on this computer `ago_ms` milliseconds ago. Only the
    /// app's own background loop reports this; the screens have no command for it. Whether and
    /// how far it keeps Auto-lock away is the session crate's rule: only with "count activity
    /// anywhere" on, and never past 60 minutes after the last activity inside the window.
    pub fn activity_elsewhere(&mut self, ago_ms: u64) {
        let now = self.clock.now_ms();
        self.session
            .activity_elsewhere(now.saturating_sub(ago_ms), now);
    }

    /// Carry out whatever is due: Auto-lock, reveal expiry, clipboard clear. Called about twice a
    /// second by the app, and at the start of every command.
    pub fn tick(&mut self) {
        let now = self.clock.now_ms();
        self.native_tick(now);
        let quick_authorized = self.quick_unlock.tick(self.clock.continuous_ms());
        if !quick_authorized && self.quick_pending.is_some() {
            self.invalidate_native();
        }
        let effects = self.session.tick(now);
        self.apply(effects);
        self.inspected.take_if(|i| now >= i.expires_at);
        self.backup_destination.take_if(|d| now >= d.expires_at);
        // Keep the wrong-credential count on disk, so a restart doesn't hand out fresh attempts.
        let failures = self.session.failed_attempts();
        if failures != self.settings.failed_attempts {
            self.settings.failed_attempts = failures;
            let _ = self.write_settings();
        }
    }

    /// Sleep, screen lock, user switch, quit, focus changes.
    pub fn os_event(&mut self, event: OsEvent) {
        if !matches!(event, OsEvent::FocusGained) {
            self.invalidate_native();
        }
        let effects = self.session.os_event(event, self.clock.now_ms());
        self.apply(effects);
        // Also when no Vault is open (e.g. a pending inspection with no Vault on this computer).
        let locks = match event {
            OsEvent::FocusLost => self.session.settings().lock_on_app_switch,
            OsEvent::FocusGained => false,
            _ => true,
        };
        if locks {
            self.forget_open_vaults();
        }
    }

    /// Latest running apps from the OS adapter (every ~2 s).
    pub fn guard_update(&mut self, apps: &[guard::RunningApp]) {
        self.detected = guard::detect(GUARD_PLATFORM, apps);
        let up = self.guard.update(&self.detected);
        if up {
            self.invalidate_native();
            self.clipboard_source = None;
        }
        let effects = self.session.set_sharing_guard(up);
        self.apply(effects);
    }

    pub fn set_lock_events_unavailable(&mut self, events: Vec<&'static str>) {
        self.lock_events_unavailable = events;
    }

    /// Latest Capture Hiding self-check (`None`: can't tell on this platform).
    pub fn set_capture_hiding_active(&mut self, active: Option<bool>) {
        self.capture_hiding_active = active;
    }

    pub fn dismiss_sharing_guard(&mut self) {
        self.guard.dismiss(&self.detected);
        let effects = self.session.set_sharing_guard(false);
        self.apply(effects);
    }

    // ---------- setup, unlock, lock ----------

    pub fn suggest_passphrase(&self) -> Zeroizing<String> {
        policy::suggest_passphrase()
    }

    pub fn assess_password(&self, password: &str) -> AssessmentView {
        assess_password(password)
    }

    /// Whether "Create my Vault" may run now (asked before the KDF is calibrated).
    pub fn create_vault_allowed(&self) -> Result<(), ApiError> {
        match self.phase {
            Phase::NoVault => Ok(()),
            _ => Err(not_now()),
        }
    }

    /// Calibrates the KDF and creates the Vault in memory; returns the Recovery Key to show once.
    /// `params`: Argon2id settings for the new Vault (the app calibrates them on this computer
    /// before calling, without holding the core).
    pub fn create_vault(
        &mut self,
        master_password: Zeroizing<String>,
        params: KdfParams,
    ) -> Result<RecoveryKeyShown, ApiError> {
        self.tick();
        if !matches!(self.phase, Phase::NoVault) {
            return Err(not_now());
        }
        check_strength(&master_password)?;
        let doc = Document::new(self.clock.unix_secs());
        let created = vault_format::create(
            &secret(&master_password),
            params,
            &self.limits,
            &doc.to_json(),
        )
        .map_err(|e| format_error(e, CredKind::Password))?;
        let shown = Zeroizing::new(created.recovery_key.expose_display().to_owned());
        self.initialize_session();
        self.set_phase(Phase::ConfirmRecoveryKit {
            open: Box::new(Open {
                vault: created.unlocked,
                doc,
            }),
            recovery_key: created.recovery_key,
            rotation: false,
        });
        Ok(RecoveryKeyShown {
            recovery_key: Some(shown),
        })
    }

    /// The owner retyped the last two groups of the Recovery Key: save and unlock.
    pub fn confirm_recovery_kit(&mut self, last_groups: &str) -> Result<(), ApiError> {
        self.tick();
        let Phase::ConfirmRecoveryKit {
            recovery_key,
            rotation,
            ..
        } = &self.phase
        else {
            return Err(not_now());
        };
        let rotation = *rotation;
        if !ends_with_groups(recovery_key.expose_display(), last_groups) {
            return Err(err("invalid_input", MSG_KIT_MISMATCH));
        }
        self.save()?;
        if rotation {
            self.note_change();
        }
        if let Phase::ConfirmRecoveryKit { open, .. } =
            std::mem::replace(&mut self.phase, Phase::Locked)
        {
            self.set_phase(Phase::Unlocked(open));
        }
        if !rotation {
            self.reset_guard();
        }
        Ok(())
    }

    pub fn unlock(&mut self, credential: CredentialInput) -> Result<(), ApiError> {
        self.tick();
        if !matches!(self.phase, Phase::Locked) {
            return Err(not_now());
        }
        let cred = parse_credential(credential)?;
        if matches!(cred, Cred::Key(_)) {
            self.invalidate_quick_unlock_registration()?;
        }
        let bytes = self.dir.read_vault().map_err(store_error)?;
        let limits = self.limits;
        let opened = attempt(&mut self.session, &*self.clock, &cred, || {
            vault_format::open(&bytes, cred.credential(), &limits)
        })
        .and_then(|(vault, body)| {
            let doc = Document::from_json(&body).map_err(notes_error)?;
            Ok(Box::new(Open { vault, doc }))
        });
        let open = opened?;
        self.start_session();
        self.set_phase(match cred {
            Cred::Password(_) => Phase::Unlocked(open),
            Cred::Key(recovery_key) => Phase::NeedsNewPassword { open, recovery_key },
        });
        self.purge_trash();
        Ok(())
    }

    /// After a Recovery Key unlock: re-wrap the password slot, save, unlock.
    pub fn set_new_password(&mut self, master_password: Zeroizing<String>) -> Result<(), ApiError> {
        self.tick();
        self.invalidate_quick_unlock_registration()?;
        let Phase::NeedsNewPassword { open, recovery_key } = &mut self.phase else {
            return Err(not_now());
        };
        check_strength(&master_password)?;
        vault_format::change_password(
            &mut open.vault,
            Credential::RecoveryKey(recovery_key),
            &secret(&master_password),
            &self.limits,
        )
        .map_err(|e| format_error(e, CredKind::RecoveryKey))?;
        let before = open.doc.to_json();
        open.doc.touch(self.clock.unix_secs());
        // If this save fails, only memory has the new slot; the owner stays here and can retry,
        // and locking throws it away.
        if let Err(e) = self.save() {
            if let (Phase::NeedsNewPassword { open, .. }, Ok(doc)) =
                (&mut self.phase, Document::from_json(&before))
            {
                open.doc = doc;
            }
            return Err(e);
        }
        self.note_change();
        if let Phase::NeedsNewPassword { open, .. } =
            std::mem::replace(&mut self.phase, Phase::Locked)
        {
            self.set_phase(Phase::Unlocked(open));
        }
        Ok(())
    }

    pub fn lock(&mut self) {
        let effects = self.session.lock();
        self.apply(effects);
        self.forget_open_vaults();
    }

    // ---------- Notes ----------

    pub fn list_notes(
        &mut self,
        filter: Filter,
        query: &str,
    ) -> Result<Vec<NoteSummary>, ApiError> {
        self.tick();
        Ok(self.open()?.doc.list(filter, query))
    }

    pub fn get_note(&mut self, id: &str) -> Result<NoteView, ApiError> {
        self.tick();
        let view = self.open()?.doc.get(id).ok_or_else(note_not_found)?;
        let effects = self.session.note_opened(id);
        self.apply(effects);
        Ok(view)
    }

    /// Wallet Kinds need the Master Password (ADR-0004).
    pub fn create_note(
        &mut self,
        input: NoteInput,
        master_password: Option<Zeroizing<String>>,
    ) -> Result<String, ApiError> {
        self.tick();
        self.open()?;
        let wallet = if input.kind.is_wallet() {
            Some(self.wallet(master_password)?)
        } else {
            None
        };
        let new = NewNote {
            kind: input.kind,
            title: input.title,
            visible: input.visible,
            hidden: input.hidden,
        };
        self.mutate(|doc, now| {
            doc.create(new, wallet.as_ref().map(|w| w as &dyn WalletCipher), now)
        })
    }

    /// Every edit of a Wallet Kind needs the Master Password, even of its title, chain or
    /// address, so nobody at an Unlocked computer can swap a recorded address (PRD story 46).
    pub fn update_note(
        &mut self,
        id: &str,
        input: NoteEditInput,
        master_password: Option<Zeroizing<String>>,
    ) -> Result<(), ApiError> {
        self.tick();
        let kind = self.note_kind(id)?;
        let wallet = if kind.is_wallet() {
            Some(self.wallet(master_password)?)
        } else {
            None
        };
        let edit = NoteEdit {
            title: Some(input.title),
            visible: input.visible,
            hidden: input.hidden,
        };
        self.mutate(|doc, now| {
            doc.update(
                id,
                edit,
                wallet.as_ref().map(|w| w as &dyn WalletCipher),
                now,
            )
        })
    }

    /// Removing a Wallet Kind needs the Master Password like every other change to one (PRD
    /// story 46): trashing it starts the 30 days after which it is gone, and deleting it for
    /// good is the one change that can't be undone at all.
    pub fn trash_note(
        &mut self,
        id: &str,
        master_password: Option<Zeroizing<String>>,
    ) -> Result<(), ApiError> {
        self.tick();
        if self.note_kind(id)?.is_wallet() {
            self.wallet(master_password)?;
        }
        let was_in_trash = self
            .open()?
            .doc
            .get(id)
            .is_some_and(|note| note.deleted_at.is_some());
        self.mutate(|doc, now| doc.trash(id, now))?;
        // If the clock went backwards while Unlocked, a newly trashed Note can look as though
        // it was already overdue at an earlier sighting. That sighting cannot cover it.
        if !was_in_trash
            && let Some(seen) = self.settings.trash_expiry_seen_at
            && self.open()?.doc.get(id).is_some_and(|note| {
                note.deleted_at
                    .is_some_and(|at| at.saturating_add(notes::TRASH_RETENTION_SECS) <= seen)
            })
        {
            self.settings.trash_expiry_seen_at = None;
            let _ = self.write_settings();
        }
        Ok(())
    }

    pub fn restore_note(&mut self, id: &str) -> Result<(), ApiError> {
        self.tick();
        self.mutate(|doc, now| doc.restore(id, now))
    }

    pub fn delete_forever(
        &mut self,
        id: &str,
        master_password: Option<Zeroizing<String>>,
    ) -> Result<(), ApiError> {
        self.tick();
        if self.note_kind(id)?.is_wallet() {
            self.wallet(master_password)?;
        }
        self.mutate(|doc, now| doc.delete_forever(id, now))
    }

    /// Needs the Master Password when a Wallet Kind is in Trash.
    pub fn empty_trash(
        &mut self,
        master_password: Option<Zeroizing<String>>,
    ) -> Result<usize, ApiError> {
        self.tick();
        let trash = self.open()?.doc.list(Filter::Trash, "");
        if trash.iter().any(|note| note.kind.is_wallet()) {
            self.wallet(master_password)?;
        }
        self.mutate(|doc, now| Ok(doc.empty_trash(now)))
    }

    pub fn set_favorite(&mut self, id: &str, favorite: bool) -> Result<(), ApiError> {
        self.tick();
        self.mutate(|doc, now| doc.set_favorite(id, favorite, now))
    }

    /// Adds `text` to the end of a Text Note's body, on a new line. The screens send only the
    /// new text: the body is joined inside the notes crate and never leaves Rust, so adding a
    /// line doesn't mean showing the Note first. Saved like any other edit; nothing is returned.
    pub fn append_text(&mut self, id: &str, text: Zeroizing<String>) -> Result<(), ApiError> {
        self.tick();
        self.mutate(|doc, now| doc.append_text(id, &text, now))
    }

    // ---------- showing & copying ----------

    /// Non-wallet Kinds: the value, shown for 30 s.
    pub fn show_field(&mut self, id: &str, field: &str) -> Result<Zeroizing<String>, ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        let value = self.open()?.doc.reveal(id, field).map_err(notes_error)?;
        self.session
            .start_reveal(id, field, RevealMode::Timed, self.clock.now_ms())
            .map_err(reveal_refused)?;
        Ok(value)
    }

    /// Wallet Kinds: Master Password again; shown while held, or for 20 s in accessible mode.
    pub fn show_wallet_field(
        &mut self,
        id: &str,
        field: &str,
        master_password: Zeroizing<String>,
        mode: ShowMode,
    ) -> Result<Zeroizing<String>, ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        if !self.note_kind(id)?.is_wallet() {
            return Err(err("invalid_input", MSG_NOT_WALLET));
        }
        let wallet = self.wallet(Some(master_password))?;
        let value = self
            .open()?
            .doc
            .reveal_wallet(id, field, &wallet)
            .map_err(notes_error)?;
        drop(wallet);
        let mode = match mode {
            ShowMode::Held => RevealMode::Held,
            ShowMode::Accessible => RevealMode::Accessible,
        };
        self.session
            .start_reveal(id, field, mode, self.clock.now_ms())
            .map_err(reveal_refused)?;
        Ok(value)
    }

    /// Wallet Kinds: show the value for 20 s and copy it, with one Master Password entry and
    /// one Wallet Key derivation. Never a held reveal: there is no button being held.
    ///
    /// The reveal comes first and counts toward the 8 different fields a minute like any
    /// other. Only once it has started is the clipboard written, so a refused reveal copies
    /// nothing. The copy is cleared after 30 s, as every Wallet Kind copy is.
    pub fn show_and_copy_wallet_field(
        &mut self,
        id: &str,
        field: &str,
        master_password: Zeroizing<String>,
    ) -> Result<Zeroizing<String>, ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        if !self.note_kind(id)?.is_wallet() {
            return Err(err("invalid_input", MSG_NOT_WALLET));
        }
        let wallet = self.wallet(Some(master_password))?;
        let value = self
            .open()?
            .doc
            .reveal_wallet(id, field, &wallet)
            .map_err(notes_error)?;
        drop(wallet);
        self.session
            .start_reveal(id, field, RevealMode::Accessible, self.clock.now_ms())
            .map_err(reveal_refused)?;
        if let Err(e) = self.copy_out(id, field, &value, true) {
            // Nothing was copied, and the value is not handed over either: nothing is shown.
            let effects = self.session.end_reveal();
            self.apply(effects);
            return Err(e);
        }
        Ok(value)
    }

    pub fn hide_field(&mut self) {
        self.invalidate_native();
        let effects = self.session.end_reveal();
        self.apply(effects);
    }

    /// More time for the value that is shown now: another 30 s for an ordinary Kind, as often
    /// as asked; another 20 s for a Wallet Kind value, three times at most, after which it has
    /// to be shown again with the Master Password. A value shown while held has no timer, and
    /// one that is no longer shown is not brought back.
    pub fn extend_reveal(&mut self) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        self.session
            .extend_reveal(self.clock.now_ms())
            .map_err(extend_refused)
    }

    /// Writes the value straight to the clipboard (never returned); cleared after the time the
    /// owner chose if unchanged, a Wallet Kind value always after 30 s.
    pub fn copy_field(
        &mut self,
        id: &str,
        field: &str,
        master_password: Option<Zeroizing<String>>,
    ) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        let wallet_kind = self.note_kind(id)?.is_wallet();
        let value = if wallet_kind {
            let wallet = self.wallet(master_password)?;
            self.open()?.doc.reveal_wallet(id, field, &wallet)
        } else {
            self.open()?.doc.reveal(id, field)
        }
        .map_err(notes_error)?;
        self.session
            .admit_hidden_field(id, field, self.clock.now_ms())
            .map_err(reveal_refused)?;
        self.copy_out(id, field, &value, wallet_kind)
    }

    /// Writes one visible field (a username, website, service, public address...) to the
    /// clipboard, privately and with the same timed clear as a Hidden Field.
    ///
    /// No Master Password, for a Wallet Kind either: its visible parts can be read by anyone at
    /// the Unlocked app. `word_count` is not copied: the app worked that number out, nobody
    /// entered it. A Hidden Field name is refused here; those go through [`AppCore::copy_field`].
    pub fn copy_visible_field(&mut self, id: &str, field: &str) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        let view = self.open()?.doc.get(id).ok_or_else(note_not_found)?;
        if field == notes::field::WORD_COUNT || !view.kind.visible_fields().contains(&field) {
            return Err(err("invalid_input", MSG_NOT_VISIBLE));
        }
        let value = view
            .visible
            .get(field)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| notes_error(NotesError::EmptyField))?;
        self.copy_out(id, field, value, false)
    }

    /// Writes one line of a Hidden Field to the clipboard, never returning it: line `line`,
    /// counted from 0 over every line of the value (split at each line feed, the way the
    /// screens split a shown value), without its line ending. Not for Wallet Kinds, whose
    /// values are copied whole and with the Master Password.
    pub fn copy_field_line(&mut self, id: &str, field: &str, line: usize) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        if self.note_kind(id)?.is_wallet() {
            return Err(err("invalid_input", MSG_LINE_NOT_WALLET));
        }
        let value = self.open()?.doc.reveal(id, field).map_err(notes_error)?;
        let chosen = value
            .split('\n')
            .nth(line)
            .ok_or_else(|| err("not_found", MSG_NO_SUCH_LINE))?;
        let chosen = chosen.strip_suffix('\r').unwrap_or(chosen);
        if chosen.is_empty() {
            return Err(err("not_found", MSG_EMPTY_LINE));
        }
        self.session
            .admit_hidden_field(id, field, self.clock.now_ms())
            .map_err(reveal_refused)?;
        self.copy_out(id, field, chosen, false)
    }

    /// Writes `NAME=value` to the clipboard for an API Key that has a variable name and a key:
    /// the line a shell or an environment file takes. The key is never returned.
    pub fn copy_field_env(&mut self, id: &str) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        let view = self.open()?.doc.get(id).ok_or_else(note_not_found)?;
        if view.kind != Kind::ApiKey {
            return Err(err("invalid_input", MSG_ENV_NOT_API_KEY));
        }
        let name = view
            .visible
            .get(notes::field::ENV_VAR)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| err("invalid_input", MSG_ENV_NO_NAME))?;
        let key = self
            .open()?
            .doc
            .reveal(id, notes::field::KEY)
            .map_err(notes_error)?;
        let line = env_line(name, &key)?;
        self.session
            .admit_hidden_field(id, notes::field::KEY, self.clock.now_ms())
            .map_err(reveal_refused)?;
        self.copy_out(id, notes::field::KEY, &line, false)
    }

    // ---------- checks ----------

    pub fn seed_wordlist(&self) -> &'static [&'static str] {
        notes::seed::wordlist()
    }

    pub fn check_seed(&self, phrase: &str) -> Result<SeedCheck, ApiError> {
        self.open()?;
        Ok(notes::seed::check(phrase))
    }

    /// `chain` is whatever the Note's chain field holds, which is free text: a name the app
    /// doesn't know is checked as "other", which means no check and no hint.
    pub fn check_private_key(
        &self,
        chain: &str,
        key: &str,
    ) -> Result<Option<&'static str>, ApiError> {
        self.open()?;
        Ok(notes::keys::check_private_key(Chain::for_check(chain), key))
    }

    // ---------- Backups & Safety Copies ----------

    /// Whether "Back up now" may open the save dialog now.
    pub fn backup_destination_allowed(&self) -> Result<(), ApiError> {
        self.open().map(|_| ())
    }

    /// The tauri layer's save dialog returned `path`. It is good for one Backup, written soon.
    pub fn set_backup_destination(&mut self, path: PathBuf) -> Result<PickedFile, ApiError> {
        self.open()?;
        let picked = self.picked(&path);
        self.backup_destination = Some(BackupDestination {
            path,
            expires_at: self.clock.now_ms().saturating_add(BACKUP_DESTINATION_MS),
        });
        Ok(picked)
    }

    /// The owner closed "Back up now" without writing (e.g. after the cloud-folder warning).
    pub fn forget_backup_destination(&mut self) {
        self.backup_destination = None;
    }

    /// Seal the current Vault into the picked destination, read it back and check it.
    ///
    /// The destination is used up whether or not the write works: every Backup goes where the
    /// owner pointed in a dialog just now, never to a path left over from earlier.
    pub fn write_backup(&mut self) -> Result<BackupWritten, ApiError> {
        self.tick();
        self.open()?;
        let dest = self
            .backup_destination
            .take()
            .ok_or_else(|| err("invalid_input", MSG_PICK_DESTINATION))?
            .path;
        let open = self.open()?;
        let bytes = vault_format::seal(&open.vault, &open.doc.to_json())
            .map_err(|e| format_error(e, CredKind::Password))?;
        let note_count = Cell::new(None);
        let verify = |written: &[u8]| {
            note_count.set(verified_count(&open.vault, written));
            note_count.get().is_some()
        };
        let commit = store::write_backup(&dest, &bytes, &verify).map_err(store_error)?;
        let written = BackupWritten {
            display_path: dest.display().to_string(),
            note_count: note_count.get().unwrap_or(0),
            durable: commit == store::CommitStatus::Durable,
        };
        // Written, flushed and read back: that is a Backup, whether or not the folder entry
        // could be flushed too (inside the Mac App Sandbox it never can).
        self.settings.last_backup_at = Some(self.clock.unix_secs());
        self.settings.oldest_unbacked_change_at = None;
        // The Backup itself is done and checked; failing to note the time isn't worth an error.
        let _ = self.write_settings();
        Ok(written)
    }

    /// Whether "Open a Backup" may open the file dialog now.
    pub fn backup_to_open_allowed(&self) -> Result<(), ApiError> {
        self.backups_allowed()
    }

    /// The tauri layer's open dialog returned `path`.
    pub fn set_backup_to_open(&mut self, path: PathBuf) -> Result<PickedFile, ApiError> {
        self.backups_allowed()?;
        self.invalidate_native();
        let picked = self.picked(&path);
        self.backup_to_open = Some(path);
        self.inspected = None;
        Ok(picked)
    }

    pub fn inspect_backup(
        &mut self,
        credential: CredentialInput,
    ) -> Result<VaultPreview, ApiError> {
        self.tick();
        self.backups_allowed()?;
        let path = self
            .backup_to_open
            .as_deref()
            .ok_or_else(|| err("invalid_input", MSG_PICK_BACKUP))?;
        let bytes = store::read_file(path).map_err(store_error)?;
        self.inspect(Source::Backup, bytes, credential)
    }

    /// The owner closed the preview without installing it: forget the decrypted copy now.
    pub fn discard_inspection(&mut self) {
        self.invalidate_native();
        self.inspected = None;
        self.backup_to_open = None;
    }

    /// Install the inspected Backup; the current Vault becomes a Safety Copy.
    pub fn replace_with_backup(&mut self) -> Result<(), ApiError> {
        self.install(Source::Backup)
    }

    /// Newest first.
    pub fn list_safety_copies(&self) -> Result<Vec<SafetyCopy>, ApiError> {
        self.backups_allowed()?;
        let copies = self.dir.safety_copies().map_err(store_error)?;
        Ok(copies
            .into_iter()
            .map(|c| SafetyCopy {
                id: c.id,
                kind: match c.kind {
                    SafetyKind::Automatic => "automatic",
                    SafetyKind::Replaced => "replaced",
                },
                created_at: c.created_at,
            })
            .collect())
    }

    pub fn inspect_safety_copy(
        &mut self,
        id: &str,
        credential: CredentialInput,
    ) -> Result<VaultPreview, ApiError> {
        self.tick();
        self.backups_allowed()?;
        let bytes = self.dir.read_safety_copy(id).map_err(store_error)?;
        self.inspect(Source::SafetyCopy, bytes, credential)
    }

    pub fn restore_safety_copy(&mut self) -> Result<(), ApiError> {
        self.install(Source::SafetyCopy)
    }

    // ---------- account care & settings ----------

    /// `rotate`: Key Rotation, and the new Recovery Key must be confirmed before it is saved.
    pub fn change_password(
        &mut self,
        current_password: Zeroizing<String>,
        new_password: Zeroizing<String>,
        rotate: bool,
    ) -> Result<RecoveryKeyShown, ApiError> {
        self.tick();
        self.open()?;
        self.invalidate_quick_unlock_registration()?;
        check_strength(&new_password)?;
        let (current, new) = (
            Cred::Password(secret(&current_password)),
            secret(&new_password),
        );
        let (now, limits) = (self.clock.now_ms(), self.limits);
        let Phase::Unlocked(open) = &mut self.phase else {
            return Err(locked());
        };

        if !rotate {
            let old_slot = open.vault.password_slot();
            attempt(&mut self.session, &*self.clock, &current, || {
                vault_format::change_password(&mut open.vault, current.credential(), &new, &limits)
            })?;
            self.session.unlocked(now); // this Vault's password: reset the wrong-password count
            let before = open.doc.to_json();
            open.doc.touch(self.clock.unix_secs());
            if let Err(e) = self.save() {
                // Put the old password back in memory too, so a later save can't switch it
                // silently. The saved copy of the slot goes back as it was: nothing to derive,
                // so nothing that can fail.
                if let Phase::Unlocked(open) = &mut self.phase {
                    open.vault.restore_password_slot(old_slot);
                    if let Ok(doc) = Document::from_json(&before) {
                        open.doc = doc;
                    }
                }
                return Err(e);
            }
            self.note_change();
            return Ok(RecoveryKeyShown { recovery_key: None });
        }

        let rotation = attempt(&mut self.session, &*self.clock, &current, || {
            vault_format::rotate_keys(&open.vault, current.credential(), &new, &limits)
        })?;
        self.session.unlocked(now);
        let vault_id = open.vault.vault_id();
        let old_wallet = Wallet {
            key: rotation.old_wallet_key,
            vault_id,
        };
        let new_wallet = Wallet {
            key: rotation.new_wallet_key,
            vault_id,
        };
        // On failure the document is untouched and the old keys stay in use.
        open.doc
            .reencrypt_wallet_fields(&old_wallet, &new_wallet, self.clock.unix_secs())
            .map_err(notes_error)?;
        let shown = Zeroizing::new(rotation.recovery_key.expose_display().to_owned());
        let effects = self.session.end_reveal();
        self.apply(effects);
        if let Phase::Unlocked(open) = std::mem::replace(&mut self.phase, Phase::Locked) {
            self.set_phase(Phase::ConfirmRecoveryKit {
                open: Box::new(Open {
                    vault: rotation.unlocked,
                    doc: open.doc,
                }),
                recovery_key: rotation.recovery_key,
                rotation: true,
            });
        }
        Ok(RecoveryKeyShown {
            recovery_key: Some(shown),
        })
    }

    /// A setting the input leaves out keeps its value. "Count activity anywhere" can only be
    /// turned on where the app can see input outside its window.
    pub fn set_settings(&mut self, settings: SettingsInput) -> Result<SettingsView, ApiError> {
        self.tick();
        self.open()?;
        let current = self.session.settings();
        let proposed = session::normalize_settings(LockSettings {
            idle_minutes: settings.idle_minutes,
            lock_on_app_switch: settings.lock_on_app_switch,
            clipboard_clear_seconds: settings
                .clipboard_clear_seconds
                .unwrap_or(current.clipboard_clear_seconds),
            count_activity_anywhere: settings
                .count_activity_anywhere
                .unwrap_or(current.count_activity_anywhere)
                && self.activity_anywhere_supported,
        });
        if proposed.idle_minutes > current.idle_minutes
            || (current.lock_on_app_switch && !proposed.lock_on_app_switch)
            || (!current.count_activity_anywhere && proposed.count_activity_anywhere)
        {
            if settings.session_generation != Some(self.session_generation) {
                return Err(not_now());
            }
            // Reuse the fresh, throttled password check; its derived Wallet Key drops immediately.
            self.wallet(settings.master_password)?;
        }
        let before = self.settings.clone();
        self.settings.idle_minutes = proposed.idle_minutes;
        self.settings.lock_on_app_switch = proposed.lock_on_app_switch;
        self.settings.clipboard_clear_seconds = proposed.clipboard_clear_seconds;
        self.settings.count_activity_anywhere = proposed.count_activity_anywhere;
        if let Err(error) = self.write_settings() {
            self.settings = before;
            return Err(error);
        }
        // Never activate an unpersisted rule, including when a write fails after authentication.
        self.session.set_settings(proposed);
        self.invalidate_native();
        Ok(self.app_status().settings)
    }

    /// The folder this computer's Vault file and Safety Copies are kept in, as text to show.
    /// Not a secret, and the same in every phase.
    pub fn vault_location(&self) -> String {
        self.dir.root().display().to_string()
    }

    pub fn mark_seed_explainer_seen(&mut self) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        self.settings.seed_explainer_seen = true;
        self.write_settings()
    }

    // ---------- internals ----------

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Lock => self.forget_open_vaults(),
                // The screens drop the shown value when status.reveal goes away.
                Effect::HideReveal => {}
                Effect::ClearClipboardIfUnchanged(count) => {
                    if !self.clipboard.clear_if(count) {
                        self.session
                            .retry_clipboard_clear(count, self.clock.now_ms());
                    } else {
                        self.clipboard_source = None;
                    }
                }
            }
        }
    }

    /// Drop every decrypted Vault (keys and document) and forget a Sharing Guard dismissal.
    fn forget_open_vaults(&mut self) {
        self.invalidate_native();
        self.clipboard_source = None;
        self.phase = match std::mem::replace(&mut self.phase, Phase::Locked) {
            Phase::NoVault
            | Phase::ConfirmRecoveryKit {
                rotation: false, ..
            } => Phase::NoVault,
            _ => Phase::Locked,
        };
        self.inspected = None;
        self.backup_to_open = None;
        self.backup_destination = None;
        self.reset_guard();
    }

    /// Every phase change drops a pending Backup or Safety Copy: its preview (and its "older"
    /// warning) described what it would have replaced before the change.
    fn set_phase(&mut self, phase: Phase) {
        self.invalidate_native();
        self.clipboard_source = None;
        self.phase = phase;
        self.inspected = None;
        self.backup_to_open = None;
        self.backup_destination = None;
    }

    /// A Vault was just opened (created, unlocked or installed): start the idle timer, and forget
    /// a Sharing Guard dismissal made while none was open.
    fn start_session(&mut self) {
        self.initialize_session();
        self.reset_guard();
    }

    // Creation keeps its current acknowledgement until the initial Kit is saved.
    fn initialize_session(&mut self) {
        self.session_generation = self.session_generation.saturating_add(1);
        self.session.unlocked(self.clock.now_ms());
    }

    fn reset_guard(&mut self) {
        self.guard.reset();
        let up = !self.guard.shield_apps().is_empty();
        let effects = self.session.set_sharing_guard(up);
        self.apply(effects);
    }

    /// The open Vault, if Unlocked.
    fn open(&self) -> Result<&Open, ApiError> {
        match &self.phase {
            Phase::Unlocked(open) => Ok(open),
            _ => Err(locked()),
        }
    }

    fn note_kind(&self, id: &str) -> Result<Kind, ApiError> {
        Ok(self.open()?.doc.get(id).ok_or_else(note_not_found)?.kind)
    }

    fn guard_check(&self) -> Result<(), ApiError> {
        if self.session.sharing_guard_up() {
            Err(err("sharing_guard_up", MSG_GUARD_UP))
        } else {
            Ok(())
        }
    }

    /// Put `text` on the clipboard privately and start the timer that clears it: the time the
    /// owner chose, or always 30 s for a Wallet Kind value (`wallet_value`). Every copy that
    /// reached the clipboard is counted for the status; a refused one is not.
    fn copy_out(
        &mut self,
        id: &str,
        field: &str,
        text: &str,
        wallet_value: bool,
    ) -> Result<(), ApiError> {
        let view = self.open()?.doc.get(id).ok_or_else(note_not_found)?;
        let source = ClipboardSource {
            title: view.title,
            kind: view.kind,
            field: field.to_owned(),
        };
        let change_count = self.clipboard.write(text)?;
        self.clipboard_source = Some(source);
        self.copies = self.copies.wrapping_add(1);
        let now = self.clock.now_ms();
        if wallet_value {
            self.session.copied_wallet_value(change_count, now);
        } else {
            self.session.copied(change_count, now);
        }
        Ok(())
    }

    /// Backups are available in NoVault, Locked and Unlocked states. Every copy still takes its
    /// credential under the wrong-credential delays.
    fn backups_allowed(&self) -> Result<(), ApiError> {
        match self.phase {
            Phase::NoVault | Phase::Locked | Phase::Unlocked(_) => Ok(()),
            _ => Err(not_now()),
        }
    }

    /// The Wallet Key for one action, from the Master Password (fresh Argon2id run).
    fn wallet(&mut self, master_password: Option<Zeroizing<String>>) -> Result<Wallet, ApiError> {
        if let Some(wallet) = self.native_wallet.take() {
            return Ok(wallet);
        }
        let password = Cred::Password(secret(
            &master_password.ok_or_else(|| err("invalid_input", MSG_NEEDS_PASSWORD))?,
        ));
        let (now, limits) = (self.clock.now_ms(), self.limits);
        let Phase::Unlocked(open) = &self.phase else {
            return Err(locked());
        };
        let key = attempt(&mut self.session, &*self.clock, &password, || {
            vault_format::unlock_wallet_key(&open.vault, password.credential(), &limits)
        })?;
        self.session.unlocked(now); // this Vault's password: reset the wrong-password count
        Ok(Wallet {
            key,
            vault_id: open.vault.vault_id(),
        })
    }

    /// Apply `change` to the open document and save; on a failed save the document goes back to
    /// what it was, so memory never differs from disk. A change that changed nothing (the notes
    /// crate then leaves `change_counter` alone) isn't saved: no new Safety Copy, no unbacked change.
    fn mutate<T>(
        &mut self,
        change: impl FnOnce(&mut Document, i64) -> Result<T, NotesError>,
    ) -> Result<T, ApiError> {
        let now = self.clock.unix_secs();
        let Phase::Unlocked(open) = &mut self.phase else {
            return Err(locked());
        };
        let (before, counter) = (open.doc.to_json(), open.doc.change_counter());
        let out = change(&mut open.doc, now).map_err(notes_error)?;
        if open.doc.change_counter() == counter {
            return Ok(out);
        }
        self.invalidate_native();
        // A pending preview's "older than current" no longer holds.
        self.inspected = None;
        if let Err(e) = self.save() {
            if let (Phase::Unlocked(open), Ok(doc)) =
                (&mut self.phase, Document::from_json(&before))
            {
                open.doc = doc;
            }
            return Err(e);
        }
        self.note_change();
        Ok(out)
    }

    /// Crash-safe save of the Vault held in the current phase, verified by reopening it.
    fn save(&mut self) -> Result<(), ApiError> {
        let open = match &self.phase {
            Phase::Unlocked(open)
            | Phase::NeedsNewPassword { open, .. }
            | Phase::ConfirmRecoveryKit { open, .. } => open,
            Phase::NoVault | Phase::Locked => return Err(locked()),
        };
        let bytes = vault_format::seal(&open.vault, &open.doc.to_json())
            .map_err(|e| format_error(e, CredKind::Password))?;
        let verify = |written: &[u8]| verified_count(&open.vault, written).is_some();
        let status = self
            .dir
            .save_vault(&bytes, &verify, self.clock.unix_secs())
            .map_err(store_error)?;
        self.save_durability_uncertain = status == store::CommitStatus::Uncertain;
        Ok(())
    }

    /// Remove Notes that have been in Trash for 30 days (after an unlock).
    ///
    /// Not on one clock reading: the first unlock that finds expired Notes only records when.
    /// At an unlock at least a day later, only Notes already expired at that first sighting go;
    /// newly expired Notes start their own wait. One mistaken future date must not let them use
    /// another Note's earlier sighting.
    fn purge_trash(&mut self) {
        let now = self.clock.unix_secs();
        let expired = match &self.phase {
            Phase::Unlocked(open) | Phase::NeedsNewPassword { open, .. } => {
                open.doc.expired_count(now) > 0
            }
            _ => return,
        };
        let seen = self.settings.trash_expiry_seen_at;
        let confirmed = seen.is_some_and(|seen| now.saturating_sub(seen) >= PURGE_CONFIRM_SECS);
        let remember = match (expired, seen) {
            (false, _) => None,
            // First sighting, or the clock has gone back since: start counting from now.
            (true, None) => Some(now),
            (true, Some(seen)) if now < seen => Some(now),
            (true, Some(seen)) => Some(seen),
        };
        if remember != seen && !(expired && confirmed) {
            self.settings.trash_expiry_seen_at = remember;
            let _ = self.write_settings();
        }
        if !(expired && confirmed) {
            return;
        }
        let before = match &self.phase {
            Phase::Unlocked(open) | Phase::NeedsNewPassword { open, .. } => open.doc.to_json(),
            _ => return,
        };
        let changed = match &mut self.phase {
            Phase::Unlocked(open) | Phase::NeedsNewPassword { open, .. } => {
                // `expired_count(now)` checked the Vault's last change against the clock.
                // Delete only the subset also overdue at the first sighting, rather than all
                // Notes overdue at this (possibly mistaken) date.
                let eligible: Vec<_> = open
                    .doc
                    .list(Filter::Trash, "")
                    .into_iter()
                    .filter(|note| {
                        note.deleted_at.is_some_and(|at| {
                            at.saturating_add(notes::TRASH_RETENTION_SECS) <= seen.unwrap_or(now)
                        })
                    })
                    .collect();
                let mut removed = false;
                for note in eligible {
                    removed |= open.doc.delete_forever(&note.id, now).is_ok();
                }
                removed
            }
            _ => false,
        };
        if changed {
            if self.save().is_ok() {
                self.purge_trash_failed = false;
                self.settings.trash_expiry_seen_at = match &self.phase {
                    Phase::Unlocked(open) | Phase::NeedsNewPassword { open, .. }
                        if open.doc.expired_count(now) > 0 =>
                    {
                        Some(now)
                    }
                    _ => None,
                };
                let _ = self.write_settings();
                self.note_change();
            } else if let (Phase::Unlocked(open) | Phase::NeedsNewPassword { open, .. }, Ok(doc)) =
                (&mut self.phase, Document::from_json(&before))
            {
                open.doc = doc;
                self.purge_trash_failed = true;
            }
        } else {
            // The earlier subset was restored or removed before this unlock. Any Notes only
            // just overdue still need their own first sighting and a later confirmation.
            self.settings.trash_expiry_seen_at = Some(now);
            let _ = self.write_settings();
        }
    }

    /// A saved change isn't in any Backup yet.
    fn note_change(&mut self) {
        if self.settings.oldest_unbacked_change_at.is_none() {
            self.settings.oldest_unbacked_change_at = Some(self.clock.unix_secs());
            let _ = self.write_settings();
        }
    }

    fn write_settings(&self) -> Result<(), ApiError> {
        let bytes =
            serde_json::to_vec(&self.settings).map_err(|_| err("internal", MSG_INTERNAL))?;
        self.dir.write_settings(&bytes).map_err(store_error)
    }

    fn picked(&self, path: &Path) -> PickedFile {
        let cloud = store::cloud_location(path, &self.home_dir, cfg!(target_os = "macos"));
        PickedFile {
            display_path: path.display().to_string(),
            cloud: cloud.map(|c| match c {
                CloudProvider::ICloudDrive => "i_cloud_drive",
                CloudProvider::MacDesktopOrDocuments => "mac_desktop_or_documents",
                CloudProvider::OneDrive => "one_drive",
                CloudProvider::Dropbox => "dropbox",
                CloudProvider::GoogleDrive => "google_drive",
            }),
        }
    }

    fn inspect(
        &mut self,
        source: Source,
        bytes: Vec<u8>,
        credential: CredentialInput,
    ) -> Result<VaultPreview, ApiError> {
        self.inspected = None;
        let cred = parse_credential(credential)?;
        let limits = self.limits;
        let (vault, body) = attempt(&mut self.session, &*self.clock, &cred, || {
            vault_format::open(&bytes, cred.credential(), &limits)
        })?;
        let doc = Document::from_json(&body).map_err(notes_error)?;
        let copy = Open { vault, doc };
        // What this computer has that the copy lacks can be told only while the Vault here is
        // readable (Unlocked) and the copy is a copy of that same Vault.
        let same_vault_here = match &self.phase {
            Phase::Unlocked(current) if current.vault.vault_id() == copy.vault.vault_id() => {
                Some(&current.doc)
            }
            _ => None,
        };
        let preview = VaultPreview {
            note_count: copy.doc.count(),
            changed_at: copy.doc.changed_at(),
            current_note_count: same_vault_here.map(Document::count),
            current_changed_at: same_vault_here.map(Document::changed_at),
            only_here: same_vault_here.map(|here| notes_only_in(here, &copy.doc)),
            older_than_current: match &self.phase {
                Phase::Unlocked(current) => Some(lacks_changes_of(&copy, current)),
                // The Vault here is Locked. The credential just given often opens it too (a
                // Safety Copy, or a Backup of this Vault): then the two can be compared after
                // all. If it doesn't, the screens say that no comparison was possible.
                Phase::Locked => self
                    .open_locked_vault(&cred)
                    .map(|c| lacks_changes_of(&copy, &c)),
                _ => None,
            },
            replaces_vault: matches!(self.phase, Phase::Locked | Phase::Unlocked(_)),
        };
        let recovery_key = match cred {
            Cred::Key(key) => Some(key),
            Cred::Password(_) => None,
        };
        // Its own time limit: in NoVault or Locked no session timer is running, and the app
        // must not go on holding a readable Vault behind a lock screen for long.
        let now = self.clock.now_ms();
        let keep_ms = match self.phase {
            Phase::Unlocked(_) => u64::from(self.session.settings().idle_minutes) * 60_000,
            _ => LOCKED_PREVIEW_MS,
        };
        self.inspected = Some(Inspected {
            source,
            bytes,
            open: Box::new(copy),
            recovery_key,
            expires_at: now.saturating_add(keep_ms),
        });
        Ok(preview)
    }

    /// This computer's Vault opened with `cred`, only to compare a copy against it; dropped by
    /// the caller at once. `None` if there is none or `cred` is not its credential (which is
    /// not counted as a wrong attempt: nobody was asked for this Vault's credential).
    fn open_locked_vault(&self, cred: &Cred) -> Option<Open> {
        let bytes = self.dir.read_vault().ok()?;
        let (vault, body) = vault_format::open(&bytes, cred.credential(), &self.limits).ok()?;
        let doc = Document::from_json(&body).ok()?;
        Some(Open { vault, doc })
    }

    fn install(&mut self, source: Source) -> Result<(), ApiError> {
        self.tick();
        self.backups_allowed()?;
        let Some(inspected) = self.inspected.take_if(|i| i.source == source) else {
            return Err(err("invalid_input", MSG_INSPECT_FIRST));
        };
        self.invalidate_quick_unlock_registration()?;
        let result = {
            let verify = |written: &[u8]| verified_count(&inspected.open.vault, written).is_some();
            self.dir
                .replace_vault(&inspected.bytes, &verify, self.clock.unix_secs())
        };
        let commit = match result {
            Ok(status) => status,
            Err(e) => {
                self.inspected = Some(inspected);
                return Err(store_error(e));
            }
        };
        // Another Vault's wrong-credential count and Trash sighting don't carry over.
        self.settings.trash_expiry_seen_at = None;
        self.save_durability_uncertain = commit == store::CommitStatus::Uncertain;
        let effects = self.session.end_reveal();
        self.apply(effects);
        self.start_session();
        let Inspected {
            open, recovery_key, ..
        } = inspected;
        self.set_phase(match recovery_key {
            Some(recovery_key) => Phase::NeedsNewPassword { open, recovery_key },
            None => Phase::Unlocked(open),
        });
        match source {
            // This computer now holds exactly what the Backup holds.
            Source::Backup => {
                self.settings.oldest_unbacked_change_at = None;
                let _ = self.write_settings();
            }
            Source::SafetyCopy => self.note_change(),
        }
        Ok(())
    }
}

/// The strength of a candidate Master Password (needs no Vault and no state).
pub fn assess_password(password: &str) -> AssessmentView {
    let a = policy::assess(password);
    AssessmentView {
        acceptable: a.acceptable,
        score: a.score,
        feedback: a.feedback,
    }
}

/// May `current` hold changes that `copy` lacks? Copies of one Vault are compared by both their
/// change counters and their clocks; different Vaults only have clocks in common.
fn lacks_changes_of(copy: &Open, current: &Open) -> bool {
    if copy.vault.vault_id() == current.vault.vault_id() {
        copy.doc.is_older_than(&current.doc)
    } else {
        copy.doc.changed_at() < current.doc.changed_at()
    }
}

/// How many Notes of `here` that are not in Trash have an id `copy` doesn't hold anywhere, its
/// Trash included: the Notes the owner would no longer have after replacing `here` with `copy`.
fn notes_only_in(here: &Document, copy: &Document) -> usize {
    let in_copy: BTreeSet<String> = [Filter::All, Filter::Trash]
        .into_iter()
        .flat_map(|filter| copy.list(filter, ""))
        .map(|note| note.id)
        .collect();
    here.list(Filter::All, "")
        .iter()
        .filter(|note| !in_copy.contains(&note.id))
        .count()
}

/// `NAME=value` for a shell or an environment file, built in one wiping buffer of exactly the
/// size needed (a growing one would leave shorter copies of the key behind as it grew).
///
/// A value made only of ASCII letters, digits and `_ @ % + = : , . / -` is written bare. Any
/// other value goes inside single quotes, where a shell takes every character as it is. That
/// leaves two things that can't be written: a single quote, which would end the quoting, and a
/// line break, which would make it more than one line.
fn env_line(name: &str, value: &str) -> Result<Zeroizing<String>, ApiError> {
    let bare = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if !bare && value.chars().any(|c| c == '\'' || is_line_break(c)) {
        return Err(err("invalid_input", MSG_ENV_CANNOT_QUOTE));
    }
    let quote = if bare { "" } else { "'" };
    let mut line = Zeroizing::new(String::with_capacity(
        name.len() + 1 + value.len() + 2 * quote.len(),
    ));
    for part in [name, "=", quote, value, quote] {
        line.push_str(part);
    }
    Ok(line)
}

/// The characters that end a line: line feed, carriage return, vertical tab, form feed, next
/// line, and the Unicode line and paragraph separators.
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// Note count of a written file, if it reopens with `vault`'s keys and parses.
fn verified_count(vault: &UnlockedVault, written: &[u8]) -> Option<usize> {
    let body = vault_format::reopen(vault, written).ok()?;
    Document::from_json(&body).ok().map(|doc| doc.count())
}

/// Run a credential check under the wrong-credential delays (Session backoff). A failure is
/// timed from when the check ends, so the second or so it took isn't taken off the delay.
fn attempt<T>(
    session: &mut Session,
    clock: &dyn Clock,
    cred: &Cred,
    check: impl FnOnce() -> Result<T, vault_format::Error>,
) -> Result<T, ApiError> {
    if let Err(wait_ms) = session.attempt_allowed(clock.now_ms()) {
        let seconds = wait_ms.div_ceil(1000);
        let unit = if seconds == 1 { "second" } else { "seconds" };
        return Err(ApiError {
            code: "retry_later",
            message: format!("Please wait {seconds} {unit}, then try again."),
        });
    }
    let kind = match cred {
        Cred::Password(_) => CredKind::Password,
        Cred::Key(_) => CredKind::RecoveryKey,
    };
    check().map_err(|e| {
        if e == vault_format::Error::WrongCredential {
            session.failed_attempt(clock.now_ms());
        }
        format_error(e, kind)
    })
}

fn parse_credential(input: CredentialInput) -> Result<Cred, ApiError> {
    match input {
        CredentialInput::MasterPassword(password) => Ok(Cred::Password(secret(&password))),
        CredentialInput::RecoveryKey(typed) => {
            RecoveryKey::parse(&typed)
                .map(Cred::Key)
                .map_err(|e| match e {
                    vault_format::RecoveryKeyError::Length => err("invalid_input", MSG_KEY_LENGTH),
                    _ => err("invalid_input", MSG_KEY_TYPO),
                })
        }
    }
}

/// Copy into an exactly-sized secret; the caller's `Zeroizing` wipes the original.
fn secret(password: &Zeroizing<String>) -> SecretString {
    SecretString::from(password.as_str())
}

fn check_strength(password: &str) -> Result<(), ApiError> {
    let assessment = policy::assess(password);
    if assessment.acceptable {
        return Ok(());
    }
    let mut message = String::from(MSG_WEAK);
    for hint in assessment.feedback {
        message.push(' ');
        message.push_str(&hint);
    }
    Err(ApiError {
        code: "weak_password",
        message,
    })
}

/// Do `typed` (ignoring case, spaces, dashes and Crockford look-alikes) equal the last two
/// 4-symbol groups of `display`?
fn ends_with_groups(display: &str, typed: &str) -> bool {
    let symbols = |s: &str| -> Zeroizing<String> {
        Zeroizing::new(
            s.chars()
                .filter(|c| *c != '-' && !c.is_whitespace())
                .map(|c| match c.to_ascii_uppercase() {
                    'O' => '0',
                    'I' | 'L' => '1',
                    c => c,
                })
                .collect(),
        )
    };
    let (key, typed) = (symbols(display), symbols(typed));
    typed.len() == 8 && key.ends_with(typed.as_str())
}

fn secs_to_ms(secs: i64) -> u64 {
    u64::try_from(secs).unwrap_or(0).saturating_mul(1000)
}

// ---------- errors ----------

#[derive(Clone, Copy)]
enum CredKind {
    Password,
    RecoveryKey,
}

const MSG_WRONG_PASSWORD: &str =
    "That password didn't unlock this Vault. Check Caps Lock and try again.";
const MSG_WRONG_KEY: &str =
    "That Recovery Key didn't unlock this Vault. Check each group against your Recovery Kit.";
const MSG_KEY_LENGTH: &str =
    "A Recovery Key has 28 characters in 7 groups of 4. Check it against your Recovery Kit.";
const MSG_KEY_TYPO: &str =
    "One of the groups doesn't look right. Check it against your Recovery Kit.";
const MSG_DAMAGED: &str = "This Vault file is damaged. Restore a Safety Copy or Backup.";
const MSG_REFUSED_SETTINGS: &str = "This Vault file has security settings this app won't accept, so it may have been tampered with. Restore a Safety Copy or Backup.";
const MSG_UNSUPPORTED: &str =
    "This Vault was made by a newer version of encrypted-note. Update the app, then try again.";
const MSG_WEAK: &str = "This password is too easy to guess.";
const MSG_LOCKED: &str = "encrypted-note is locked. Unlock it, then try again.";
const MSG_NOT_NOW: &str = "That isn't available right now.";
const MSG_GUARD_UP: &str =
    "A screen-sharing or recording app is running, so nothing can be shown or copied right now.";
const MSG_NOTE_NOT_FOUND: &str = "That Note couldn't be found. It may have been deleted.";
const MSG_NEEDS_PASSWORD: &str =
    "Enter your Master Password to do this with a Seed Phrase or Private Key.";
const MSG_NOT_WALLET: &str = "Only Seed Phrases and Private Keys are shown this way.";
const MSG_EMPTY_TITLE: &str = "Give the Note a title.";
const MSG_EMPTY_FIELD: &str = "That field is empty.";
const MSG_BAD_EXPIRES: &str =
    "Write the expiry date as year-month-day, like 2027-01-31, using a day that exists.";
const MSG_BAD_ENV_VAR: &str = "A variable name can use letters, digits and underscores, can't start with a digit, and can be up to 128 characters long.";
const MSG_BAD_PURPOSE: &str = "Keep the purpose to one line of up to 200 characters.";
const MSG_ONE_LINE_ONLY: &str =
    "One of the fields has a line break or a tab in it. Keep each of them to one line.";
const MSG_BAD_KEY_ID: &str = "The key ID has a line break or a tab in it. Keep it to one line.";
const MSG_BAD_URL: &str =
    "The management URL has a line break or a tab in it. Keep it to one line.";
const MSG_BAD_ADDRESS: &str =
    "The public address has a line break or a tab in it. Keep it to one line.";
const MSG_BAD_DERIVATION_PATH: &str =
    "The derivation path has a line break or a tab in it. Keep it to one line.";
const MSG_NOTHING_TO_ADD: &str = "Type the text to add first.";
const MSG_NOT_VISIBLE: &str =
    "Only the parts of a Note that are always visible can be copied this way.";
const MSG_LINE_NOT_WALLET: &str =
    "A Seed Phrase or Private Key is copied whole, with your Master Password.";
const MSG_NO_SUCH_LINE: &str = "That line isn't there any more. Show the value again, then copy.";
const MSG_EMPTY_LINE: &str = "That line is empty.";
const MSG_ENV_NOT_API_KEY: &str = "Only an API Key can be copied as NAME=value.";
const MSG_ENV_NO_NAME: &str = "Add a variable name to this API Key first.";
const MSG_ENV_CANNOT_QUOTE: &str = "This key has a line break or a single quote in it, so it can't be written as NAME=value. Copy the key by itself.";
const MSG_NOTHING_SHOWN: &str = "That value isn't shown any more. Show it again.";
const MSG_HELD_NO_TIMER: &str =
    "A value shown while the button is held stays for as long as it is held.";
const MSG_EXTEND_LIMIT: &str =
    "This value has been given more time three times. Enter your Master Password to show it again.";
const MSG_KIT_MISMATCH: &str =
    "That doesn't match the end of your Recovery Key. Check what you wrote down and try again.";
const MSG_PICK_DESTINATION: &str = "Choose where to save the Backup first.";
const MSG_PICK_BACKUP: &str = "Choose a Backup file first.";
const MSG_INSPECT_FIRST: &str = "Open it with its Master Password or Recovery Key first.";
const MSG_ALREADY_OPEN: &str =
    "encrypted-note is already open. Use the window that's already open.";
const MSG_NO_VAULT: &str = "There's no Vault on this computer yet.";
const MSG_SAFETY_COPY_GONE: &str = "That Safety Copy isn't there any more.";
const MSG_VERIFY_FAILED: &str =
    "The file was written but didn't read back correctly, so it wasn't used. Nothing was changed.";
const MSG_BACKUP_OVERWRITE_NOT_ATOMIC: &str = "This Backup already exists, and encrypted-note can't safely replace it with the access the file picker granted. Choose a new file name. Nothing was changed.";
const MSG_INTERNAL: &str = "Something went wrong inside encrypted-note. Nothing was changed.";
const MSG_NO_MEMORY: &str = "There isn't enough free memory to check that right now. Close other apps and try again. Nothing was changed.";
const MSG_TOO_LARGE: &str =
    "That file is far larger than any Vault, so encrypted-note left it alone. Nothing was changed.";

fn err(code: &'static str, message: &str) -> ApiError {
    ApiError {
        code,
        message: message.to_owned(),
    }
}

fn locked() -> ApiError {
    err("locked", MSG_LOCKED)
}

fn not_now() -> ApiError {
    err("invalid_input", MSG_NOT_NOW)
}

fn note_not_found() -> ApiError {
    err("not_found", MSG_NOTE_NOT_FOUND)
}

fn reveal_refused(refused: RevealRefused) -> ApiError {
    match refused {
        RevealRefused::Locked => locked(),
        RevealRefused::SharingGuardUp => err("sharing_guard_up", MSG_GUARD_UP),
        RevealRefused::TooMany(wait_ms) => {
            let seconds = wait_ms.div_ceil(1000);
            let unit = if seconds == 1 { "second" } else { "seconds" };
            ApiError {
                code: "too_many",
                message: format!(
                    "Too many different values were shown or copied in a short time. Wait {seconds} {unit}, then try again."
                ),
            }
        }
    }
}

fn extend_refused(refused: ExtendRefused) -> ApiError {
    match refused {
        ExtendRefused::NothingShown => err("not_found", MSG_NOTHING_SHOWN),
        ExtendRefused::Held => err("invalid_input", MSG_HELD_NO_TIMER),
        ExtendRefused::LimitReached => err("invalid_input", MSG_EXTEND_LIMIT),
    }
}

fn format_error(e: vault_format::Error, kind: CredKind) -> ApiError {
    use vault_format::Error as E;
    match e {
        E::WrongCredential => match kind {
            CredKind::Password => err("wrong_credential", MSG_WRONG_PASSWORD),
            CredKind::RecoveryKey => err("wrong_credential", MSG_WRONG_KEY),
        },
        E::Malformed(_) | E::Damaged => err("damaged", MSG_DAMAGED),
        E::KdfOutOfLimits => err("damaged", MSG_REFUSED_SETTINGS),
        E::OutOfMemory => err("io", MSG_NO_MEMORY),
        E::UnsupportedVersion(_) => err("unsupported", MSG_UNSUPPORTED),
        E::Randomness | E::SlotCheckFailed => err("internal", MSG_INTERNAL),
    }
}

fn notes_error(e: NotesError) -> ApiError {
    match e {
        NotesError::NotFound => note_not_found(),
        NotesError::UnknownField(name) => ApiError {
            code: "invalid_input",
            message: format!("This kind of Note has no field called \"{name}\"."),
        },
        NotesError::NeedsWalletKey => err("invalid_input", MSG_NEEDS_PASSWORD),
        NotesError::EmptyField => err("not_found", MSG_EMPTY_FIELD),
        NotesError::EmptyTitle => err("invalid_input", MSG_EMPTY_TITLE),
        NotesError::BadWordCount(n) => ApiError {
            code: "invalid_input",
            message: format!("A Seed Phrase has 12 or 24 words. This one has {n}."),
        },
        NotesError::InvalidValue(name) => err("invalid_input", invalid_value_message(&name)),
        NotesError::WalletCipher | NotesError::Corrupt(_) => err("damaged", MSG_DAMAGED),
        NotesError::UnsupportedSchema(_) => err("unsupported", MSG_UNSUPPORTED),
        NotesError::Randomness => err("internal", MSG_INTERNAL),
    }
}

/// What to say when a value breaks the rule for its field, naming the field the way the
/// Editor labels it. The notes crate reports the field's name only, so the value itself can
/// never end up in the message.
fn invalid_value_message(field_name: &str) -> &'static str {
    match field_name {
        notes::field::EXPIRES => MSG_BAD_EXPIRES,
        notes::field::ENV_VAR => MSG_BAD_ENV_VAR,
        notes::field::PURPOSE => MSG_BAD_PURPOSE,
        notes::field::BODY => MSG_NOTHING_TO_ADD,
        notes::field::KEY_ID => MSG_BAD_KEY_ID,
        notes::field::URL => MSG_BAD_URL,
        notes::field::ADDRESS => MSG_BAD_ADDRESS,
        notes::field::DERIVATION_PATH => MSG_BAD_DERIVATION_PATH,
        _ => MSG_ONE_LINE_ONLY,
    }
}

fn store_error(e: StoreError) -> ApiError {
    match e {
        StoreError::AlreadyOpen => err("already_open", MSG_ALREADY_OPEN),
        StoreError::NoVault => err("not_found", MSG_NO_VAULT),
        StoreError::NotFound => err("not_found", MSG_SAFETY_COPY_GONE),
        StoreError::VerifyFailed => err("io", MSG_VERIFY_FAILED),
        StoreError::BackupOverwriteNotAtomic => err("io", MSG_BACKUP_OVERWRITE_NOT_ATOMIC),
        StoreError::TooLarge => err("io", MSG_TOO_LARGE),
        StoreError::Io(e) => ApiError {
            code: "io",
            message: format!(
                "encrypted-note couldn't read or write a file ({e}). Nothing was changed."
            ),
        },
    }
}
