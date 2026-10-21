//! One native credential panel, one frozen action, and a checked commit after off-lock KDF work.
//! None of these types is a WebView message or a reusable approval.

use super::*;

const OPERATION_MS: u64 = 2 * 60_000;
type ActionDescription = (&'static str, Option<String>, Option<Kind>, bool);

pub enum NativeAction {
    CreateVault {
        params: KdfParams,
    },
    ConfirmRecoveryKit,
    Unlock {
        recovery: bool,
    },
    SetNewPassword,
    CreateNote(NoteInput),
    UpdateNote {
        id: String,
        input: NoteEditInput,
    },
    TrashNote {
        id: String,
    },
    DeleteForever {
        id: String,
    },
    EmptyTrash,
    ShowWalletField {
        id: String,
        field: String,
        mode: ShowMode,
    },
    ShowAndCopyWalletField {
        id: String,
        field: String,
    },
    CopyField {
        id: String,
        field: String,
    },
    InspectBackup {
        recovery: bool,
    },
    InspectSafetyCopy {
        id: String,
        recovery: bool,
    },
    ChangePassword {
        rotate: bool,
    },
    SetSettings(SettingsInput),
    EnrollQuickUnlock,
}

/// Rust-native text only. No Debug, Serialize or Clone implementation.
pub enum NativeCredentials {
    None,
    Password(Zeroizing<String>),
    RecoveryKey(Zeroizing<String>),
    NewPassword {
        password: Zeroizing<String>,
        confirmation: Zeroizing<String>,
    },
    PasswordChange {
        current: Zeroizing<String>,
        password: Zeroizing<String>,
        confirmation: Zeroizing<String>,
    },
    KitConfirmation(Zeroizing<String>),
}

pub struct NativeOperation {
    pub id: u64,
    pub action: &'static str,
    pub target: Option<String>,
    pub kind: Option<Kind>,
    pub field: Option<String>,
    pub requires_credentials: bool,
}

/// The coordinator must handle RecoveryKit natively and return only the action's public result.
pub enum NativeResult {
    Unit,
    NoteId(String),
    Count(usize),
    Field(Zeroizing<String>),
    Preview(VaultPreview),
    Settings(SettingsView),
    RecoveryKit(RecoveryKeyShown),
    QuickUnlockEnrollment(QuickUnlockEnrollment),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PhaseTag {
    NoVault,
    Kit,
    Locked,
    Reset,
    Unlocked,
}

enum InspectionSource {
    Backup(PathBuf),
    SafetyCopy(String),
}

struct Snapshot {
    epoch: u64,
    generation: u64,
    phase: PhaseTag,
    counter: Option<u64>,
    disk: Option<Vec<u8>>,
    inspection: Option<(InspectionSource, Vec<u8>)>,
    deadline: u64,
}

pub(super) struct Pending {
    id: u64,
    snapshot: Snapshot,
    action: Option<NativeAction>,
    requires_credentials: bool,
}

pub struct NativeJob {
    id: u64,
    action: NativeAction,
    credentials: NativeCredentials,
    bytes: Option<Vec<u8>>,
    current_locked_bytes: Option<Vec<u8>>,
    recovery_key: Option<RecoveryKey>,
    limits: Limits,
    now_unix: i64,
    authenticates: bool,
}

pub struct NativeCompletion {
    id: u64,
    master_binding: Option<vault_format::QuickUnlockBinding>,
    action: NativeAction,
    prepared: Result<Prepared, JobError>,
    authenticates: bool,
}

struct JobError {
    error: ApiError,
    wrong_credential: bool,
}

enum Prepared {
    QuickUnlock(vault_format::QuickUnlockKey),
    Plain,
    Kit(Zeroizing<String>),
    Created {
        open: Box<Open>,
        recovery_key: RecoveryKey,
    },
    Unlocked {
        open: Box<Open>,
        recovery_key: Option<RecoveryKey>,
    },
    Reset(Box<Open>),
    PasswordChanged(Box<Open>),
    Rotated {
        open: Box<Open>,
        recovery_key: RecoveryKey,
    },
    Wallet(Wallet),
    Inspected {
        bytes: Vec<u8>,
        open: Box<Open>,
        recovery_key: Option<RecoveryKey>,
        locked_current: Option<Box<Open>>,
    },
}

impl AppCore {
    /// Validate and freeze arguments before opening native controls. An overlap never queues.
    pub fn begin_native(&mut self, mut action: NativeAction) -> Result<NativeOperation, ApiError> {
        self.tick();
        if self.native_pending.is_some() || self.quick_pending.is_some() {
            return Err(not_now());
        }
        self.guard_check()?;
        let effects = self.session.end_reveal();
        self.apply(effects);
        let (label, target, kind, requires_credentials) = self.native_description(&action)?;
        if matches!(
            action,
            NativeAction::ChangePassword { .. }
                | NativeAction::SetNewPassword
                | NativeAction::Unlock { recovery: true }
                | NativeAction::EnrollQuickUnlock
        ) {
            self.invalidate_quick_unlock_registration()?;
            if matches!(action, NativeAction::EnrollQuickUnlock) {
                // Even an orphan item is removed only after disabled policy is durable.
                self.write_settings()?;
                self.quick_cleanup_needed = true;
            }
        }
        // A legacy internal caller cannot smuggle credential text into a frozen settings input.
        if let NativeAction::SetSettings(input) = &mut action {
            input.master_password = None;
        }
        let inspection = match &action {
            NativeAction::InspectBackup { .. } => {
                let path = self
                    .backup_to_open
                    .clone()
                    .ok_or_else(|| err("invalid_input", MSG_PICK_BACKUP))?;
                let bytes = store::read_file(&path).map_err(store_error)?;
                Some((InspectionSource::Backup(path), bytes))
            }
            NativeAction::InspectSafetyCopy { id, .. } => Some((
                InspectionSource::SafetyCopy(id.clone()),
                self.dir.read_safety_copy(id).map_err(store_error)?,
            )),
            _ => None,
        };
        let disk = self.native_disk()?;
        self.native_sequence = self.native_sequence.checked_add(1).ok_or_else(not_now)?;
        let id = self.native_sequence;
        let field = match &action {
            NativeAction::ShowWalletField { field, .. }
            | NativeAction::ShowAndCopyWalletField { field, .. }
            | NativeAction::CopyField { field, .. } => Some(field.clone()),
            _ => None,
        };
        self.native_pending = Some(Pending {
            id,
            snapshot: Snapshot {
                epoch: self.native_epoch,
                generation: self.session_generation,
                phase: self.native_phase(),
                counter: self.native_counter(),
                disk,
                inspection,
                deadline: self.clock.now_ms().saturating_add(OPERATION_MS),
            },
            action: Some(action),
            requires_credentials,
        });
        Ok(NativeOperation {
            id,
            action: label,
            target,
            kind,
            field,
            requires_credentials,
        })
    }

    /// Move only encrypted snapshots and native-owned credential buffers into a worker.
    pub fn prepare_native(
        &mut self,
        id: u64,
        credentials: NativeCredentials,
    ) -> Result<NativeJob, ApiError> {
        self.tick();
        self.validate_native(id)?;
        let pending = self.native_pending.as_ref().ok_or_else(not_now)?;
        let action = pending.action.as_ref().ok_or_else(not_now)?;
        let authenticates = native_authenticates(action, pending.requires_credentials);
        if authenticates {
            attempt_allowed(&self.session, self.clock.now_ms())?;
        }
        validate_credentials(action, &credentials, pending.requires_credentials)?;
        let bytes = if matches!(action, NativeAction::EnrollQuickUnlock) {
            pending.snapshot.disk.clone()
        } else if let Some((_, bytes)) = &pending.snapshot.inspection {
            Some(bytes.clone())
        } else {
            match &self.phase {
                Phase::Locked => pending.snapshot.disk.clone(),
                Phase::Unlocked(open) | Phase::NeedsNewPassword { open, .. } => Some(
                    vault_format::seal(&open.vault, &open.doc.to_json())
                        .map_err(|e| format_error(e, CredKind::Password))?,
                ),
                _ => None,
            }
        };
        let recovery_key = if let Phase::NeedsNewPassword { recovery_key, .. } = &self.phase {
            Some(
                RecoveryKey::parse(recovery_key.expose_display())
                    .map_err(|_| err("internal", MSG_INTERNAL))?,
            )
        } else {
            None
        };
        let current_locked_bytes =
            if pending.snapshot.inspection.is_some() && matches!(self.phase, Phase::Locked) {
                pending.snapshot.disk.clone()
            } else {
                None
            };
        let now_unix = self.clock.unix_secs();
        let action = self
            .native_pending
            .as_mut()
            .and_then(|p| p.action.take())
            .ok_or_else(not_now)?;
        Ok(NativeJob {
            id,
            action,
            credentials,
            bytes,
            current_locked_bytes,
            recovery_key,
            limits: self.limits,
            now_unix,
            authenticates,
        })
    }

    /// A request ID is cancellation-only; it cannot approve or run an action.
    pub fn cancel_native(&mut self, id: u64) {
        if self.native_pending.as_ref().is_some_and(|p| p.id == id) {
            self.invalidate_native();
        }
    }

    pub fn cancel_all_native(&mut self) {
        self.invalidate_native();
    }

    /// Rust-only stale native-readout guard; never an approval or credential.
    pub fn native_operation_epoch(&self) -> u64 {
        self.native_epoch
    }

    pub fn native_pending_id(&self) -> Option<u64> {
        self.native_pending.as_ref().map(|p| p.id)
    }

    pub(super) fn invalidate_native(&mut self) {
        self.quick_unlock.cancel_pending();
        self.quick_pending = None;
        self.native_pending = None;
        self.native_wallet = None;
        self.native_epoch = self.native_epoch.saturating_add(1);
    }

    /// Reject stale work before failed-attempt accounting or any plaintext/clipboard/disk effect.
    pub fn finish_native(
        &mut self,
        completion: NativeCompletion,
    ) -> Result<NativeResult, ApiError> {
        self.tick();
        self.validate_native(completion.id)?;
        let master_disk = self
            .native_pending
            .as_ref()
            .and_then(|pending| pending.snapshot.disk.clone());
        self.native_pending = None;
        let prepared = match completion.prepared {
            Ok(prepared) => prepared,
            Err(failure) => {
                if failure.wrong_credential {
                    self.session.failed_attempt(self.clock.now_ms());
                    self.tick();
                }
                return Err(failure.error);
            }
        };
        if completion.authenticates
            && !matches!(
                completion.action,
                NativeAction::InspectBackup { .. } | NativeAction::InspectSafetyCopy { .. }
            )
        {
            self.session.unlocked(self.clock.now_ms());
        }
        if native_current_master(&completion.action, completion.authenticates)
            && let (Some(bytes), Some(binding)) = (master_disk, completion.master_binding)
        {
            self.quick_master_verified(&bytes, binding);
        }
        let result = self.commit_native(completion.action, prepared);
        // Wallet authentication is scoped to exactly one call, including every early error.
        self.native_wallet = None;
        result
    }

    fn native_phase(&self) -> PhaseTag {
        match self.phase {
            Phase::NoVault => PhaseTag::NoVault,
            Phase::ConfirmRecoveryKit { .. } => PhaseTag::Kit,
            Phase::Locked => PhaseTag::Locked,
            Phase::NeedsNewPassword { .. } => PhaseTag::Reset,
            Phase::Unlocked(_) => PhaseTag::Unlocked,
        }
    }

    fn native_counter(&self) -> Option<u64> {
        match &self.phase {
            Phase::Unlocked(open)
            | Phase::NeedsNewPassword { open, .. }
            | Phase::ConfirmRecoveryKit { open, .. } => Some(open.doc.change_counter()),
            _ => None,
        }
    }

    fn native_disk(&self) -> Result<Option<Vec<u8>>, ApiError> {
        if self.dir.has_vault() {
            self.dir.read_vault().map(Some).map_err(store_error)
        } else {
            Ok(None)
        }
    }

    pub(super) fn native_tick(&mut self, now: u64) {
        if self
            .native_pending
            .as_ref()
            .is_some_and(|pending| now >= pending.snapshot.deadline)
            || self.quick_pending.is_some_and(|deadline| now >= deadline)
        {
            self.invalidate_native();
        }
    }

    fn validate_native(&mut self, id: u64) -> Result<(), ApiError> {
        if self
            .native_pending
            .as_ref()
            .is_none_or(|pending| pending.id != id)
        {
            return Err(not_now());
        }
        let valid = self.native_pending.as_ref().is_some_and(|pending| {
            pending.id == id
                && pending.snapshot.epoch == self.native_epoch
                && pending.snapshot.generation == self.session_generation
                && pending.snapshot.phase == self.native_phase()
                && pending.snapshot.counter == self.native_counter()
                && self.clock.now_ms() < pending.snapshot.deadline
                && !self.session.sharing_guard_up()
        });
        if !valid {
            self.invalidate_native();
            return Err(not_now());
        }
        let pending = self.native_pending.as_ref().ok_or_else(not_now)?;
        if self.native_disk()? != pending.snapshot.disk {
            self.invalidate_native();
            return Err(not_now());
        }
        if let Some((source, expected)) = &pending.snapshot.inspection {
            let actual = match source {
                InspectionSource::Backup(path) => {
                    if self.backup_to_open.as_ref() != Some(path) {
                        self.invalidate_native();
                        return Err(not_now());
                    }
                    store::read_file(path).map_err(store_error)?
                }
                InspectionSource::SafetyCopy(id) => {
                    self.dir.read_safety_copy(id).map_err(store_error)?
                }
            };
            if &actual != expected {
                self.invalidate_native();
                return Err(not_now());
            }
        }
        Ok(())
    }

    fn native_description(&self, action: &NativeAction) -> Result<ActionDescription, ApiError> {
        let note = |id: &str| self.open()?.doc.get(id).ok_or_else(note_not_found);
        Ok(match action {
            NativeAction::EnrollQuickUnlock => {
                self.open()?;
                if !self.quick_unlock_available || self.clock.continuous_ms().is_none() {
                    return Err(err(
                        "invalid_input",
                        "Touch ID is unavailable. Use your Master Password.",
                    ));
                }
                ("enroll_quick_unlock", None, None, true)
            }
            NativeAction::CreateVault { .. } => {
                self.create_vault_allowed()?;
                ("create_vault", None, None, true)
            }
            NativeAction::ConfirmRecoveryKit => {
                if !matches!(self.phase, Phase::ConfirmRecoveryKit { .. }) {
                    return Err(not_now());
                }
                ("confirm_recovery_kit", None, None, true)
            }
            NativeAction::Unlock { .. } => {
                if !matches!(self.phase, Phase::Locked) {
                    return Err(not_now());
                }
                ("unlock", None, None, true)
            }
            NativeAction::SetNewPassword => {
                if !matches!(self.phase, Phase::NeedsNewPassword { .. }) {
                    return Err(not_now());
                }
                ("set_new_password", None, None, true)
            }
            NativeAction::CreateNote(input) => {
                self.open()?;
                (
                    "create_note",
                    Some(input.title.clone()),
                    Some(input.kind),
                    input.kind.is_wallet(),
                )
            }
            NativeAction::UpdateNote { id, .. } => {
                let note = note(id)?;
                (
                    "update_note",
                    Some(note.title),
                    Some(note.kind),
                    note.kind.is_wallet(),
                )
            }
            NativeAction::TrashNote { id } => {
                let note = note(id)?;
                (
                    "trash_note",
                    Some(note.title),
                    Some(note.kind),
                    note.kind.is_wallet(),
                )
            }
            NativeAction::DeleteForever { id } => {
                let note = note(id)?;
                (
                    "delete_forever",
                    Some(note.title),
                    Some(note.kind),
                    note.kind.is_wallet(),
                )
            }
            NativeAction::EmptyTrash => {
                let trash = self.open()?.doc.list(Filter::Trash, "");
                (
                    "empty_trash",
                    None,
                    None,
                    trash.iter().any(|note| note.kind.is_wallet()),
                )
            }
            NativeAction::ShowWalletField { id, field, .. }
            | NativeAction::ShowAndCopyWalletField { id, field } => {
                let note = note(id)?;
                if !note.kind.is_wallet() {
                    return Err(err("invalid_input", MSG_NOT_WALLET));
                }
                if !note.kind.hidden_fields().contains(&field.as_str()) {
                    return Err(notes_error(NotesError::UnknownField(field.clone())));
                }
                let label = if matches!(action, NativeAction::ShowWalletField { .. }) {
                    "show_wallet_field"
                } else {
                    "show_and_copy_wallet_field"
                };
                (label, Some(note.title), Some(note.kind), true)
            }
            NativeAction::CopyField { id, field } => {
                let note = note(id)?;
                if !note.kind.hidden_fields().contains(&field.as_str()) {
                    return Err(notes_error(NotesError::UnknownField(field.clone())));
                }
                (
                    "copy_field",
                    Some(note.title),
                    Some(note.kind),
                    note.kind.is_wallet(),
                )
            }
            NativeAction::InspectBackup { .. } => {
                self.backups_allowed()?;
                ("inspect_backup", None, None, true)
            }
            NativeAction::InspectSafetyCopy { .. } => {
                self.backups_allowed()?;
                ("inspect_safety_copy", None, None, true)
            }
            NativeAction::ChangePassword { rotate } => {
                self.open()?;
                (
                    if *rotate {
                        "rotate_keys"
                    } else {
                        "change_password"
                    },
                    None,
                    None,
                    true,
                )
            }
            NativeAction::SetSettings(input) => {
                self.open()?;
                let current = self.session.settings();
                let proposed = self.native_settings(input);
                let weaker = proposed.idle_minutes > current.idle_minutes
                    || (current.lock_on_app_switch && !proposed.lock_on_app_switch)
                    || (!current.count_activity_anywhere && proposed.count_activity_anywhere);
                if weaker && input.session_generation != Some(self.session_generation) {
                    return Err(not_now());
                }
                ("set_settings", None, None, weaker)
            }
        })
    }

    fn native_settings(&self, input: &SettingsInput) -> LockSettings {
        let current = self.session.settings();
        session::normalize_settings(LockSettings {
            idle_minutes: input.idle_minutes,
            lock_on_app_switch: input.lock_on_app_switch,
            clipboard_clear_seconds: input
                .clipboard_clear_seconds
                .unwrap_or(current.clipboard_clear_seconds),
            count_activity_anywhere: input
                .count_activity_anywhere
                .unwrap_or(current.count_activity_anywhere)
                && self.activity_anywhere_supported,
        })
    }

    fn commit_native(
        &mut self,
        action: NativeAction,
        prepared: Prepared,
    ) -> Result<NativeResult, ApiError> {
        match (action, prepared) {
            (NativeAction::EnrollQuickUnlock, Prepared::QuickUnlock(key)) => self
                .prepare_quick_unlock_enrollment(key)
                .map(NativeResult::QuickUnlockEnrollment),
            (NativeAction::CreateVault { .. }, Prepared::Created { open, recovery_key }) => {
                let shown = Zeroizing::new(recovery_key.expose_display().to_owned());
                self.start_session();
                self.set_phase(Phase::ConfirmRecoveryKit {
                    open,
                    recovery_key,
                    rotation: false,
                });
                Ok(NativeResult::RecoveryKit(RecoveryKeyShown {
                    recovery_key: Some(shown),
                }))
            }
            (NativeAction::ConfirmRecoveryKit, Prepared::Kit(groups)) => {
                self.confirm_recovery_kit(&groups)?;
                Ok(NativeResult::Unit)
            }
            (NativeAction::Unlock { .. }, Prepared::Unlocked { open, recovery_key }) => {
                self.start_session();
                self.set_phase(match recovery_key {
                    Some(recovery_key) => Phase::NeedsNewPassword { open, recovery_key },
                    None => Phase::Unlocked(open),
                });
                self.purge_trash();
                Ok(NativeResult::Unit)
            }
            (NativeAction::SetNewPassword, Prepared::Reset(open)) => {
                // Keep the original recovery state intact until the candidate is durably saved.
                let before = std::mem::replace(&mut self.phase, Phase::Unlocked(open));
                if let Err(error) = self.save() {
                    self.phase = before;
                    return Err(error);
                }
                self.note_change();
                let phase = std::mem::replace(&mut self.phase, Phase::Locked);
                self.set_phase(phase);
                Ok(NativeResult::Unit)
            }
            (NativeAction::ChangePassword { rotate: false }, Prepared::PasswordChanged(open)) => {
                let before = std::mem::replace(&mut self.phase, Phase::Unlocked(open));
                if let Err(error) = self.save() {
                    self.phase = before;
                    return Err(error);
                }
                self.note_change();
                self.invalidate_native();
                Ok(NativeResult::RecoveryKit(RecoveryKeyShown {
                    recovery_key: None,
                }))
            }
            (
                NativeAction::ChangePassword { rotate: true },
                Prepared::Rotated { open, recovery_key },
            ) => {
                let shown = Zeroizing::new(recovery_key.expose_display().to_owned());
                self.set_phase(Phase::ConfirmRecoveryKit {
                    open,
                    recovery_key,
                    rotation: true,
                });
                Ok(NativeResult::RecoveryKit(RecoveryKeyShown {
                    recovery_key: Some(shown),
                }))
            }
            (
                action @ (NativeAction::InspectBackup { .. }
                | NativeAction::InspectSafetyCopy { .. }),
                Prepared::Inspected {
                    bytes,
                    open,
                    recovery_key,
                    locked_current,
                },
            ) => {
                let source = if matches!(action, NativeAction::InspectBackup { .. }) {
                    Source::Backup
                } else {
                    Source::SafetyCopy
                };
                let same = match &self.phase {
                    Phase::Unlocked(current)
                        if current.vault.vault_id() == open.vault.vault_id() =>
                    {
                        Some(&current.doc)
                    }
                    _ => None,
                };
                let preview = VaultPreview {
                    note_count: open.doc.count(),
                    changed_at: open.doc.changed_at(),
                    current_note_count: same.map(Document::count),
                    current_changed_at: same.map(Document::changed_at),
                    only_here: same.map(|here| notes_only_in(here, &open.doc)),
                    older_than_current: match &self.phase {
                        Phase::Unlocked(current) => Some(lacks_changes_of(&open, current)),
                        _ => locked_current
                            .as_ref()
                            .map(|current| lacks_changes_of(&open, current)),
                    },
                    replaces_vault: matches!(self.phase, Phase::Locked | Phase::Unlocked(_)),
                };
                let keep_ms = if matches!(self.phase, Phase::Unlocked(_)) {
                    u64::from(self.session.settings().idle_minutes) * 60_000
                } else {
                    LOCKED_PREVIEW_MS
                };
                self.inspected = Some(Inspected {
                    source,
                    bytes,
                    open,
                    recovery_key,
                    expires_at: self.clock.now_ms().saturating_add(keep_ms),
                });
                Ok(NativeResult::Preview(preview))
            }
            (action, prepared @ (Prepared::Plain | Prepared::Wallet(_))) => {
                if let Prepared::Wallet(wallet) = prepared {
                    self.native_wallet = Some(wallet);
                }
                match action {
                    NativeAction::CreateNote(input) => {
                        self.create_note(input, None).map(NativeResult::NoteId)
                    }
                    NativeAction::UpdateNote { id, input } => self
                        .update_note(&id, input, None)
                        .map(|_| NativeResult::Unit),
                    NativeAction::TrashNote { id } => {
                        self.trash_note(&id, None).map(|_| NativeResult::Unit)
                    }
                    NativeAction::DeleteForever { id } => {
                        self.delete_forever(&id, None).map(|_| NativeResult::Unit)
                    }
                    NativeAction::EmptyTrash => self.empty_trash(None).map(NativeResult::Count),
                    NativeAction::ShowWalletField { id, field, mode } => self
                        .show_wallet_field(&id, &field, Zeroizing::new(String::new()), mode)
                        .map(NativeResult::Field),
                    NativeAction::ShowAndCopyWalletField { id, field } => self
                        .show_and_copy_wallet_field(&id, &field, Zeroizing::new(String::new()))
                        .map(NativeResult::Field),
                    NativeAction::CopyField { id, field } => self
                        .copy_field(&id, &field, None)
                        .map(|_| NativeResult::Unit),
                    NativeAction::SetSettings(input) => {
                        self.set_settings(input).map(NativeResult::Settings)
                    }
                    _ => Err(not_now()),
                }
            }
            _ => Err(not_now()),
        }
    }
}

impl NativeJob {
    /// Run on a worker without holding CoreState. All returned plaintext stays Rust-owned.
    pub fn run(mut self) -> NativeCompletion {
        let master_binding = if native_current_master(&self.action, self.authenticates) {
            self.bytes
                .as_deref()
                .and_then(|bytes| vault_format::quick_unlock_binding(bytes, &self.limits).ok())
        } else {
            None
        };
        let prepared = self.derive();
        NativeCompletion {
            id: self.id,
            master_binding,
            action: self.action,
            prepared,
            authenticates: self.authenticates,
        }
    }

    fn derive(&mut self) -> Result<Prepared, JobError> {
        let credentials = std::mem::replace(&mut self.credentials, NativeCredentials::None);
        let limits = self.limits;
        let bytes = self.bytes.take();
        match (&self.action, credentials) {
            (
                NativeAction::CreateVault { params },
                NativeCredentials::NewPassword {
                    password,
                    confirmation,
                },
            ) => {
                checked_new_password(&password, &confirmation).map_err(job_input)?;
                let doc = Document::new(self.now_unix);
                let created =
                    vault_format::create(&secret(&password), *params, &limits, &doc.to_json())
                        .map_err(|e| job_format(e, CredKind::Password))?;
                Ok(Prepared::Created {
                    open: Box::new(Open {
                        vault: created.unlocked,
                        doc,
                    }),
                    recovery_key: created.recovery_key,
                })
            }
            (NativeAction::ConfirmRecoveryKit, NativeCredentials::KitConfirmation(groups)) => {
                Ok(Prepared::Kit(groups))
            }
            (
                NativeAction::SetNewPassword,
                NativeCredentials::NewPassword {
                    password,
                    confirmation,
                },
            ) => {
                checked_new_password(&password, &confirmation).map_err(job_input)?;
                let key = self
                    .recovery_key
                    .take()
                    .ok_or_else(|| job_input(not_now()))?;
                let (mut vault, body) = vault_format::open(
                    bytes.as_deref().ok_or_else(|| job_input(not_now()))?,
                    Credential::RecoveryKey(&key),
                    &limits,
                )
                .map_err(|e| job_format(e, CredKind::RecoveryKey))?;
                vault_format::change_password(
                    &mut vault,
                    Credential::RecoveryKey(&key),
                    &secret(&password),
                    &limits,
                )
                .map_err(|e| job_format(e, CredKind::RecoveryKey))?;
                let mut doc = Document::from_json(&body).map_err(|e| job_input(notes_error(e)))?;
                doc.touch(self.now_unix);
                Ok(Prepared::Reset(Box::new(Open { vault, doc })))
            }
            (
                NativeAction::ChangePassword { rotate },
                NativeCredentials::PasswordChange {
                    current,
                    password,
                    confirmation,
                },
            ) => {
                checked_new_password(&password, &confirmation).map_err(job_input)?;
                let current = secret(&current);
                let (mut vault, body) = vault_format::open(
                    bytes.as_deref().ok_or_else(|| job_input(not_now()))?,
                    Credential::MasterPassword(&current),
                    &limits,
                )
                .map_err(|e| job_format(e, CredKind::Password))?;
                let mut doc = Document::from_json(&body).map_err(|e| job_input(notes_error(e)))?;
                if *rotate {
                    let rotation = vault_format::rotate_keys(
                        &vault,
                        Credential::MasterPassword(&current),
                        &secret(&password),
                        &limits,
                    )
                    .map_err(|e| job_format(e, CredKind::Password))?;
                    let id = vault.vault_id();
                    doc.reencrypt_wallet_fields(
                        &Wallet {
                            key: rotation.old_wallet_key,
                            vault_id: id,
                        },
                        &Wallet {
                            key: rotation.new_wallet_key,
                            vault_id: id,
                        },
                        self.now_unix,
                    )
                    .map_err(|e| job_input(notes_error(e)))?;
                    Ok(Prepared::Rotated {
                        open: Box::new(Open {
                            vault: rotation.unlocked,
                            doc,
                        }),
                        recovery_key: rotation.recovery_key,
                    })
                } else {
                    vault_format::change_password(
                        &mut vault,
                        Credential::MasterPassword(&current),
                        &secret(&password),
                        &limits,
                    )
                    .map_err(|e| job_format(e, CredKind::Password))?;
                    doc.touch(self.now_unix);
                    Ok(Prepared::PasswordChanged(Box::new(Open { vault, doc })))
                }
            }
            (action, NativeCredentials::Password(password)) => {
                self.open_with(action, bytes, Cred::Password(secret(&password)))
            }
            (action, NativeCredentials::RecoveryKey(typed)) => {
                let cred =
                    parse_credential(CredentialInput::RecoveryKey(typed)).map_err(job_input)?;
                self.open_with(action, bytes, cred)
            }
            (_, NativeCredentials::None) if !self.authenticates => Ok(Prepared::Plain),
            _ => Err(job_input(not_now())),
        }
    }

    fn open_with(
        &self,
        action: &NativeAction,
        bytes: Option<Vec<u8>>,
        cred: Cred,
    ) -> Result<Prepared, JobError> {
        let kind = if matches!(cred, Cred::Password(_)) {
            CredKind::Password
        } else {
            CredKind::RecoveryKey
        };
        let (vault, body) = vault_format::open(
            bytes.as_deref().ok_or_else(|| job_input(not_now()))?,
            cred.credential(),
            &self.limits,
        )
        .map_err(|e| job_format(e, kind))?;
        if matches!(action, NativeAction::EnrollQuickUnlock) {
            let key = vault_format::enroll_quick_unlock(
                &vault,
                bytes.as_deref().ok_or_else(|| job_input(not_now()))?,
                &self.limits,
            )
            .map_err(|e| job_format(e, CredKind::Password))?;
            return Ok(Prepared::QuickUnlock(key));
        }
        if matches!(action, NativeAction::Unlock { .. }) {
            let doc = Document::from_json(&body).map_err(|e| job_input(notes_error(e)))?;
            return Ok(Prepared::Unlocked {
                open: Box::new(Open { vault, doc }),
                recovery_key: match cred {
                    Cred::Key(key) => Some(key),
                    _ => None,
                },
            });
        }
        if matches!(
            action,
            NativeAction::InspectBackup { .. } | NativeAction::InspectSafetyCopy { .. }
        ) {
            let doc = Document::from_json(&body).map_err(|e| job_input(notes_error(e)))?;
            let locked_current = self.current_locked_bytes.as_ref().and_then(|current| {
                let (vault, body) =
                    vault_format::open(current, cred.credential(), &self.limits).ok()?;
                Some(Box::new(Open {
                    vault,
                    doc: Document::from_json(&body).ok()?,
                }))
            });
            return Ok(Prepared::Inspected {
                bytes: bytes.ok_or_else(|| job_input(not_now()))?,
                open: Box::new(Open { vault, doc }),
                locked_current,
                recovery_key: match cred {
                    Cred::Key(key) => Some(key),
                    _ => None,
                },
            });
        }
        let key = vault_format::unlock_wallet_key(&vault, cred.credential(), &self.limits)
            .map_err(|e| job_format(e, kind))?;
        Ok(Prepared::Wallet(Wallet {
            key,
            vault_id: vault.vault_id(),
        }))
    }
}

fn native_authenticates(action: &NativeAction, required: bool) -> bool {
    required
        && !matches!(
            action,
            NativeAction::CreateVault { .. }
                | NativeAction::ConfirmRecoveryKit
                | NativeAction::SetNewPassword
        )
}

fn validate_credentials(
    action: &NativeAction,
    credentials: &NativeCredentials,
    required: bool,
) -> Result<(), ApiError> {
    let valid = match action {
        NativeAction::CreateVault { .. } | NativeAction::SetNewPassword => {
            matches!(credentials, NativeCredentials::NewPassword { .. })
        }
        NativeAction::ConfirmRecoveryKit => {
            matches!(credentials, NativeCredentials::KitConfirmation(_))
        }
        NativeAction::ChangePassword { .. } => {
            matches!(credentials, NativeCredentials::PasswordChange { .. })
        }
        NativeAction::Unlock { recovery }
        | NativeAction::InspectBackup { recovery }
        | NativeAction::InspectSafetyCopy { recovery, .. } => {
            if *recovery {
                matches!(credentials, NativeCredentials::RecoveryKey(_))
            } else {
                matches!(credentials, NativeCredentials::Password(_))
            }
        }
        _ if required => matches!(credentials, NativeCredentials::Password(_)),
        _ => matches!(credentials, NativeCredentials::None),
    };
    if valid { Ok(()) } else { Err(not_now()) }
}

fn checked_new_password(password: &str, confirmation: &str) -> Result<(), ApiError> {
    if password != confirmation {
        return Err(err(
            "invalid_input",
            "The passwords don't match. Please try again.",
        ));
    }
    check_strength(password)
}

fn attempt_allowed(session: &Session, now: u64) -> Result<(), ApiError> {
    session.attempt_allowed(now).map_err(|wait_ms| {
        let seconds = wait_ms.div_ceil(1000);
        let unit = if seconds == 1 { "second" } else { "seconds" };
        err(
            "retry_later",
            &format!("Please wait {seconds} {unit}, then try again."),
        )
    })
}

fn job_input(error: ApiError) -> JobError {
    JobError {
        error,
        wrong_credential: false,
    }
}
fn job_format(error: vault_format::Error, kind: CredKind) -> JobError {
    JobError {
        wrong_credential: error == vault_format::Error::WrongCredential,
        error: format_error(error, kind),
    }
}

fn native_current_master(action: &NativeAction, authenticates: bool) -> bool {
    authenticates
        && !matches!(
            action,
            NativeAction::Unlock { recovery: true }
                | NativeAction::InspectBackup { .. }
                | NativeAction::InspectSafetyCopy { .. }
                | NativeAction::ChangePassword { .. }
                | NativeAction::SetNewPassword
                | NativeAction::EnrollQuickUnlock
        )
}
