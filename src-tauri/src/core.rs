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
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use notes::keys::Chain;
use notes::seed::SeedCheck;
use notes::{
    Document, Filter, Kind, NewNote, NoteEdit, NoteSummary, NoteView, NotesError, WalletCipher,
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use session::{Effect, LockSettings, OsEvent, RevealMode, RevealRefused, Session};
use store::{CloudProvider, SafetyKind, StoreError, VaultDir};
use vault_format::{Credential, KdfParams, Limits, RecoveryKey, UnlockedVault, WalletKey};
use zeroize::Zeroizing;

// ---------- injected dependencies ----------

/// Time, injected so tests can move it.
pub trait Clock: Send {
    /// Monotonic milliseconds (session timers).
    fn now_ms(&self) -> u64;
    /// Wall-clock Unix seconds (timestamps stored in the Vault and settings).
    fn unix_secs(&self) -> i64;
}

/// The system clipboard. Implementations never read what's on it.
pub trait Clipboard: Send {
    /// Put `text` on the clipboard privately; return the clipboard's change count after the write.
    fn write(&mut self, text: &str) -> Result<u64, ApiError>;
    /// Clear the clipboard, but only if its change count still equals `change_count`.
    fn clear_if(&mut self, change_count: u64);
}

pub struct Config {
    /// The data folder (`.../Vault.noindex`).
    pub data_dir: PathBuf,
    /// The owner's home folder, for spotting cloud-synced Backup destinations.
    pub home_dir: PathBuf,
    pub limits: Limits,
    pub clock: Box<dyn Clock>,
    pub clipboard: Box<dyn Clipboard>,
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

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct SettingsInput {
    pub idle_minutes: u32,
    pub lock_on_app_switch: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppStatus {
    pub phase: &'static str,
    pub platform: &'static str,
    pub capture_hiding_active: Option<bool>,
    pub sharing_guard: GuardView,
    pub lock_in_ms: Option<u64>,
    pub clipboard_clear_in_ms: Option<u64>,
    pub reveal: Option<RevealView>,
    pub failed_attempts: u32,
    pub retry_in_ms: Option<u64>,
    pub show_recovery_hint: bool,
    pub backup: BackupView,
    pub settings: SettingsView,
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
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct VaultPreview {
    pub note_count: usize,
    pub changed_at: i64,
    pub older_than_current: Option<bool>,
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

/// A Backup or Safety Copy decrypted by `inspect_*`, waiting for "Replace". Dropped after the
/// idle time (whatever the phase), on anything that locks, on any phase change and on any edit.
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
}

impl Default for StoredSettings {
    fn default() -> Self {
        StoredSettings {
            idle_minutes: session::DEFAULT_IDLE_MINUTES,
            lock_on_app_switch: false,
            seed_explainer_seen: false,
            last_backup_at: None,
            oldest_unbacked_change_at: None,
        }
    }
}

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
    session: Session,
    guard: guard::GuardState,
    detected: Vec<&'static str>,
    capture_hiding_active: Option<bool>,
    phase: Phase,
    settings: StoredSettings,
    backup_destination: Option<PathBuf>,
    backup_to_open: Option<PathBuf>,
    inspected: Option<Inspected>,
    /// An unlock found the Vault file damaged: allow restoring a Safety Copy or Backup while Locked.
    vault_damaged: bool,
}

impl AppCore {
    /// Opens the data folder (taking the single-instance lock) and reads the settings.
    pub fn new(config: Config) -> Result<AppCore, ApiError> {
        let dir = VaultDir::open(&config.data_dir).map_err(store_error)?;
        // A missing or unreadable settings file just means defaults; it holds nothing precious.
        let settings: StoredSettings = dir
            .read_settings()
            .ok()
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let session = Session::new(LockSettings {
            idle_minutes: settings.idle_minutes,
            lock_on_app_switch: settings.lock_on_app_switch,
        });
        let phase = if dir.has_vault() {
            Phase::Locked
        } else {
            Phase::NoVault
        };
        Ok(AppCore {
            dir,
            home_dir: config.home_dir,
            limits: config.limits,
            clock: config.clock,
            clipboard: config.clipboard,
            session,
            guard: guard::GuardState::new(),
            detected: Vec::new(),
            capture_hiding_active: None,
            phase,
            settings,
            backup_destination: None,
            backup_to_open: None,
            inspected: None,
            vault_damaged: false,
        })
    }

    // ---------- status & events ----------

    pub fn app_status(&self) -> AppStatus {
        let now = self.clock.now_ms();
        let shield = self.guard.shield_apps();
        let lock = self.session.settings();
        AppStatus {
            phase: match self.phase {
                Phase::NoVault => "no_vault",
                Phase::ConfirmRecoveryKit { .. } => "confirm_recovery_kit",
                Phase::Locked => "locked",
                Phase::NeedsNewPassword { .. } => "needs_new_password",
                Phase::Unlocked(_) => "unlocked",
            },
            platform: PLATFORM,
            capture_hiding_active: self.capture_hiding_active,
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
            reveal: self.session.current_reveal().map(|r| RevealView {
                note_id: r.note_id.clone(),
                field: r.field.clone(),
                expires_in_ms: r.expires_at.map(|at| at.saturating_sub(now)),
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
            settings: SettingsView {
                idle_minutes: lock.idle_minutes,
                lock_on_app_switch: lock.lock_on_app_switch,
                seed_explainer_seen: self.settings.seed_explainer_seen,
            },
        }
    }

    /// Wall-clock Unix seconds from the injected clock (e.g. for a Backup's default file name).
    pub fn unix_secs(&self) -> i64 {
        self.clock.unix_secs()
    }

    /// Owner activity (keeps Auto-lock away).
    pub fn activity(&mut self) {
        self.session.activity(self.clock.now_ms());
    }

    /// Carry out whatever is due: Auto-lock, reveal expiry, clipboard clear. Called about twice a
    /// second by the app, and at the start of every command.
    pub fn tick(&mut self) {
        let now = self.clock.now_ms();
        let effects = self.session.tick(now);
        self.apply(effects);
        self.inspected.take_if(|i| now >= i.expires_at);
    }

    /// Sleep, screen lock, user switch, quit, focus changes.
    pub fn os_event(&mut self, event: OsEvent) {
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
        let effects = self.session.set_sharing_guard(up);
        self.apply(effects);
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
        let a = policy::assess(password);
        AssessmentView {
            acceptable: a.acceptable,
            score: a.score,
            feedback: a.feedback,
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
        self.start_session();
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
        Ok(())
    }

    pub fn unlock(&mut self, credential: CredentialInput) -> Result<(), ApiError> {
        self.tick();
        if !matches!(self.phase, Phase::Locked) {
            return Err(not_now());
        }
        let cred = parse_credential(credential)?;
        let bytes = self.dir.read_vault().map_err(store_error)?;
        let now = self.clock.now_ms();
        let limits = self.limits;
        let opened = attempt(&mut self.session, now, &cred, || {
            vault_format::open(&bytes, cred.credential(), &limits)
        })
        .and_then(|(vault, body)| {
            let doc = Document::from_json(&body).map_err(notes_error)?;
            Ok(Box::new(Open { vault, doc }))
        });
        let open = opened.inspect_err(|e| self.vault_damaged |= e.code == "damaged")?;
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
        // If this save fails, only memory has the new slot; the owner stays here and can retry,
        // and locking throws it away.
        self.save()?;
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

    pub fn trash_note(&mut self, id: &str) -> Result<(), ApiError> {
        self.tick();
        self.mutate(|doc, now| doc.trash(id, now))
    }

    pub fn restore_note(&mut self, id: &str) -> Result<(), ApiError> {
        self.tick();
        self.mutate(|doc, now| doc.restore(id, now))
    }

    pub fn delete_forever(&mut self, id: &str) -> Result<(), ApiError> {
        self.tick();
        self.mutate(|doc, now| doc.delete_forever(id, now))
    }

    pub fn empty_trash(&mut self) -> Result<usize, ApiError> {
        self.tick();
        self.mutate(|doc, now| Ok(doc.empty_trash(now)))
    }

    pub fn set_favorite(&mut self, id: &str, favorite: bool) -> Result<(), ApiError> {
        self.tick();
        self.mutate(|doc, now| doc.set_favorite(id, favorite, now))
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

    pub fn hide_field(&mut self) {
        let effects = self.session.end_reveal();
        self.apply(effects);
    }

    /// Writes the value straight to the clipboard (never returned); cleared after 30 s if unchanged.
    pub fn copy_field(
        &mut self,
        id: &str,
        field: &str,
        master_password: Option<Zeroizing<String>>,
    ) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        self.guard_check()?;
        let value = if self.note_kind(id)?.is_wallet() {
            let wallet = self.wallet(master_password)?;
            self.open()?.doc.reveal_wallet(id, field, &wallet)
        } else {
            self.open()?.doc.reveal(id, field)
        }
        .map_err(notes_error)?;
        let change_count = self.clipboard.write(&value)?;
        self.session.copied(change_count, self.clock.now_ms());
        Ok(())
    }

    // ---------- checks ----------

    pub fn seed_wordlist(&self) -> &'static [&'static str] {
        notes::seed::wordlist()
    }

    pub fn check_seed(&self, phrase: &str) -> Result<SeedCheck, ApiError> {
        self.open()?;
        Ok(notes::seed::check(phrase))
    }

    pub fn check_private_key(
        &self,
        chain: Chain,
        key: &str,
    ) -> Result<Option<&'static str>, ApiError> {
        self.open()?;
        Ok(notes::keys::check_private_key(chain, key))
    }

    // ---------- Backups & Safety Copies ----------

    /// Whether "Back up now" may open the save dialog now.
    pub fn backup_destination_allowed(&self) -> Result<(), ApiError> {
        self.open().map(|_| ())
    }

    /// The tauri layer's save dialog returned `path`.
    pub fn set_backup_destination(&mut self, path: PathBuf) -> Result<PickedFile, ApiError> {
        self.open()?;
        let picked = self.picked(&path);
        self.backup_destination = Some(path);
        Ok(picked)
    }

    /// Seal the current Vault into the picked destination, read it back and check it.
    pub fn write_backup(&mut self) -> Result<BackupWritten, ApiError> {
        self.tick();
        let open = self.open()?;
        let dest = self
            .backup_destination
            .as_deref()
            .ok_or_else(|| err("invalid_input", MSG_PICK_DESTINATION))?;
        let bytes = vault_format::seal(&open.vault, &open.doc.to_json())
            .map_err(|e| format_error(e, CredKind::Password))?;
        let note_count = Cell::new(None);
        let verify = |written: &[u8]| {
            note_count.set(verified_count(&open.vault, written));
            note_count.get().is_some()
        };
        store::write_backup(dest, &bytes, &verify).map_err(store_error)?;
        let written = BackupWritten {
            display_path: dest.display().to_string(),
            note_count: note_count.get().unwrap_or(0),
        };
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
        let bytes = std::fs::read(path).map_err(|e| store_error(StoreError::Io(e)))?;
        self.inspect(Source::Backup, bytes, credential)
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
            attempt(&mut self.session, now, &current, || {
                vault_format::change_password(&mut open.vault, current.credential(), &new, &limits)
            })?;
            self.session.unlocked(now); // this Vault's password: reset the wrong-password count
            if let Err(e) = self.save() {
                // Put the old password back in memory too, so a later save can't switch it silently.
                if let (Phase::Unlocked(open), Cred::Password(old)) = (&mut self.phase, &current) {
                    let _ = vault_format::change_password(
                        &mut open.vault,
                        Credential::MasterPassword(&new),
                        old,
                        &limits,
                    );
                }
                return Err(e);
            }
            self.note_change();
            return Ok(RecoveryKeyShown { recovery_key: None });
        }

        let rotation = attempt(&mut self.session, now, &current, || {
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

    pub fn set_settings(&mut self, settings: SettingsInput) -> Result<(), ApiError> {
        self.tick();
        self.open()?;
        self.session.set_settings(LockSettings {
            idle_minutes: settings.idle_minutes,
            lock_on_app_switch: settings.lock_on_app_switch,
        });
        let applied = self.session.settings();
        self.settings.idle_minutes = applied.idle_minutes;
        self.settings.lock_on_app_switch = applied.lock_on_app_switch;
        self.write_settings()
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
                Effect::ClearClipboardIfUnchanged(count) => self.clipboard.clear_if(count),
            }
        }
    }

    /// Drop every decrypted Vault (keys and document) and forget a Sharing Guard dismissal.
    fn forget_open_vaults(&mut self) {
        self.phase = match std::mem::replace(&mut self.phase, Phase::Locked) {
            Phase::NoVault
            | Phase::ConfirmRecoveryKit {
                rotation: false, ..
            } => Phase::NoVault,
            _ => Phase::Locked,
        };
        self.inspected = None;
        self.reset_guard();
    }

    /// Every phase change drops a pending Backup or Safety Copy: its preview (and its "older"
    /// warning) described what it would have replaced before the change.
    fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.inspected = None;
        self.backup_to_open = None;
    }

    /// A Vault was just opened (created, unlocked or installed): start the idle timer, and forget
    /// a Sharing Guard dismissal made while none was open.
    fn start_session(&mut self) {
        self.session.unlocked(self.clock.now_ms());
        self.reset_guard();
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

    /// While Locked only once an unlock found the Vault file damaged ("Restore a Safety Copy or
    /// Backup"). Opening one still takes its credential, under the wrong-credential delays.
    fn backups_allowed(&self) -> Result<(), ApiError> {
        match self.phase {
            Phase::NoVault | Phase::Unlocked(_) => Ok(()),
            Phase::Locked if self.vault_damaged => Ok(()),
            Phase::Locked => Err(locked()),
            _ => Err(not_now()),
        }
    }

    /// The Wallet Key for one action, from the Master Password (fresh Argon2id run).
    fn wallet(&mut self, master_password: Option<Zeroizing<String>>) -> Result<Wallet, ApiError> {
        let password = Cred::Password(secret(
            &master_password.ok_or_else(|| err("invalid_input", MSG_NEEDS_PASSWORD))?,
        ));
        let (now, limits) = (self.clock.now_ms(), self.limits);
        let Phase::Unlocked(open) = &self.phase else {
            return Err(locked());
        };
        let key = attempt(&mut self.session, now, &password, || {
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
    fn save(&self) -> Result<(), ApiError> {
        let open = match &self.phase {
            Phase::Unlocked(open)
            | Phase::NeedsNewPassword { open, .. }
            | Phase::ConfirmRecoveryKit { open, .. } => open,
            Phase::NoVault | Phase::Locked => return Err(locked()),
        };
        let bytes = vault_format::seal(&open.vault, &open.doc.to_json())
            .map_err(|e| format_error(e, CredKind::Password))?;
        let verify = |written: &[u8]| verified_count(&open.vault, written).is_some();
        self.dir
            .save_vault(&bytes, &verify, self.clock.unix_secs())
            .map_err(store_error)
    }

    /// Remove Notes that have been in Trash for 30 days (after an unlock).
    fn purge_trash(&mut self) {
        let now = self.clock.unix_secs();
        if let Phase::Unlocked(open) | Phase::NeedsNewPassword { open, .. } = &mut self.phase
            && open.doc.purge_expired(now) > 0
            && self.save().is_ok()
        {
            self.note_change();
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
        let (now, limits) = (self.clock.now_ms(), self.limits);
        let (vault, body) = attempt(&mut self.session, now, &cred, || {
            vault_format::open(&bytes, cred.credential(), &limits)
        })?;
        let doc = Document::from_json(&body).map_err(notes_error)?;
        let preview = VaultPreview {
            note_count: doc.count(),
            changed_at: doc.changed_at(),
            older_than_current: match &self.phase {
                Phase::Unlocked(current) => Some(doc.is_older_than(&current.doc)),
                _ => None,
            },
        };
        let recovery_key = match cred {
            Cred::Key(key) => Some(key),
            Cred::Password(_) => None,
        };
        // Its own idle limit: in NoVault or Locked no session timer is running.
        let idle_ms = u64::from(self.session.settings().idle_minutes) * 60_000;
        self.inspected = Some(Inspected {
            source,
            bytes,
            open: Box::new(Open { vault, doc }),
            recovery_key,
            expires_at: now.saturating_add(idle_ms),
        });
        Ok(preview)
    }

    fn install(&mut self, source: Source) -> Result<(), ApiError> {
        self.tick();
        self.backups_allowed()?;
        let Some(inspected) = self.inspected.take_if(|i| i.source == source) else {
            return Err(err("invalid_input", MSG_INSPECT_FIRST));
        };
        let result = {
            let verify = |written: &[u8]| verified_count(&inspected.open.vault, written).is_some();
            self.dir
                .replace_vault(&inspected.bytes, &verify, self.clock.unix_secs())
        };
        if let Err(e) = result {
            self.inspected = Some(inspected);
            return Err(store_error(e));
        }
        let effects = self.session.end_reveal();
        self.apply(effects);
        self.vault_damaged = false;
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

/// Note count of a written file, if it reopens with `vault`'s keys and parses.
fn verified_count(vault: &UnlockedVault, written: &[u8]) -> Option<usize> {
    let body = vault_format::reopen(vault, written).ok()?;
    Document::from_json(&body).ok().map(|doc| doc.count())
}

/// Run a credential check under the wrong-credential delays (Session backoff).
fn attempt<T>(
    session: &mut Session,
    now: u64,
    cred: &Cred,
    check: impl FnOnce() -> Result<T, vault_format::Error>,
) -> Result<T, ApiError> {
    if let Err(wait_ms) = session.attempt_allowed(now) {
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
            session.failed_attempt(now);
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
const MSG_INTERNAL: &str = "Something went wrong inside encrypted-note. Nothing was changed.";

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
        E::UnsupportedVersion(_) => err("unsupported", MSG_UNSUPPORTED),
        E::Randomness => err("internal", MSG_INTERNAL),
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
        NotesError::WalletCipher | NotesError::Corrupt(_) => err("damaged", MSG_DAMAGED),
        NotesError::Randomness => err("internal", MSG_INTERNAL),
    }
}

fn store_error(e: StoreError) -> ApiError {
    match e {
        StoreError::AlreadyOpen => err("already_open", MSG_ALREADY_OPEN),
        StoreError::NoVault => err("not_found", MSG_NO_VAULT),
        StoreError::NotFound => err("not_found", MSG_SAFETY_COPY_GONE),
        StoreError::VerifyFailed => err("io", MSG_VERIFY_FAILED),
        StoreError::Io(e) => ApiError {
            code: "io",
            message: format!(
                "encrypted-note couldn't read or write a file ({e}). Nothing was changed."
            ),
        },
    }
}
