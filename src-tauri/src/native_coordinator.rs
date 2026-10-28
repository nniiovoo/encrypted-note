//! One native operation at a time. Credentials, Wallet readouts and Recovery Kits stay in Rust.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError, TryLockError, mpsc};
use std::time::{Duration, Instant};

use notes::Kind;
use tauri::{AppHandle, Emitter, Manager};
use zeroize::Zeroizing;

use crate::commands::{self, CoreState, STATUS_EVENT};
use crate::core::native::{NativeAction, NativeCredentials, NativeOperation, NativeResult};
use crate::core::{ApiError, AppCore, AppStatus, ShowMode};
use crate::native_credential::{
    self, Authorization, Prompt, PromptError, Reply, Reveal, RevealMode,
};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct CoordinatorState {
    sequence: AtomicU64,
    current: AtomicU64,
    active: Mutex<Option<Active>>,
}

#[derive(Clone)]
struct Active {
    token: u64,
    id: u64,
    deadline: Instant,
    stage: Stage,
}

#[derive(Clone)]
enum Stage {
    Calibration {
        epoch: u64,
        generation: u64,
        phase: &'static str,
    },
    Entry,
    RecoveryKit {
        epoch: u64,
    },
    Reveal(RevealIdentity),
}

#[derive(Clone)]
struct RevealIdentity {
    epoch: u64,
    generation: u64,
    note_id: String,
    field: String,
}

impl RevealIdentity {
    fn matches(&self, status: &AppStatus, epoch: u64) -> bool {
        self.epoch == epoch && self.same_field(status)
    }

    fn same_field(&self, status: &AppStatus) -> bool {
        status.phase == "unlocked"
            && !status.sharing_guard.up
            && status.session_generation == self.generation
            && status
                .reveal
                .as_ref()
                .is_some_and(|shown| shown.note_id == self.note_id && shown.field == self.field)
    }
}

impl CoordinatorState {
    fn reserve(&self, deadline: Instant) -> Result<u64, ApiError> {
        let token = self
            .sequence
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
            .map_err(|_| unavailable())?
            + 1;
        self.current
            .compare_exchange(0, token, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| busy())?;
        if !self.publish(token, 0, deadline, Stage::Entry) {
            return Err(cancelled());
        }
        Ok(token)
    }

    fn live(&self, token: u64) -> bool {
        self.current.load(Ordering::SeqCst) == token
    }

    fn publish(&self, token: u64, id: u64, deadline: Instant, stage: Stage) -> bool {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.live(token) {
            return false;
        }
        *active = Some(Active {
            token,
            id,
            deadline,
            stage,
        });
        true
    }

    fn snapshot(&self, token: u64) -> Option<Active> {
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|active| active.token == token && self.live(token))
            .cloned()
    }

    fn try_snapshot(&self, token: u64) -> Snapshot {
        match self.active.try_lock() {
            Ok(active) => active
                .as_ref()
                .filter(|active| active.token == token && self.live(token))
                .cloned()
                .map_or(Snapshot::Denied, Snapshot::Active),
            Err(TryLockError::WouldBlock) => Snapshot::Retry,
            Err(TryLockError::Poisoned(_)) => Snapshot::Denied,
        }
    }

    fn stop(&self, token: u64) -> Option<Active> {
        // Keep the old panel ID until the next lease can publish its metadata.
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        self.current
            .compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        if active.as_ref().is_some_and(|active| active.token == token) {
            active.take()
        } else {
            None
        }
    }

    fn idle(&self) -> bool {
        self.current.load(Ordering::SeqCst) == 0
    }
}

enum Snapshot {
    Active(Active),
    Retry,
    Denied,
}

struct Lease {
    app: AppHandle,
    token: u64,
    deadline: Instant,
}

impl Drop for Lease {
    fn drop(&mut self) {
        cancel_token(&self.app, self.token);
    }
}

#[derive(Clone, Copy)]
enum EntryKind {
    Password,
    Recovery,
    NewPassword,
    PasswordChange,
}

fn reserve(app: &AppHandle) -> Result<Lease, ApiError> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let token = app.state::<CoordinatorState>().reserve(deadline)?;
    Ok(Lease {
        app: app.clone(),
        token,
        deadline,
    })
}

/// Reserve before calibration so a cancelled setup cannot reopen its password panel later.
pub async fn create(app: AppHandle) -> Result<NativeResult, ApiError> {
    let lease = reserve(&app)?;
    let token = lease.token;
    let deadline = lease.deadline;
    let prepare_app = app.clone();
    core_work(&app, move |core| {
        check_live(&prepare_app, token)?;
        core.create_vault_allowed()?;
        let status = core.app_status();
        if !prepare_app.state::<CoordinatorState>().publish(
            token,
            0,
            deadline,
            Stage::Calibration {
                epoch: core.native_operation_epoch(),
                generation: status.session_generation,
                phase: status.phase,
            },
        ) {
            return Err(cancelled());
        }
        Ok(())
    })
    .await?;
    let params = tauri::async_runtime::spawn_blocking(|| {
        vault_format::calibrate(&vault_format::Limits::PRODUCTION, vault_format::measure_kdf)
    })
    .await
    .map_err(|_| unavailable())?;
    check_live(&app, token)?;
    run_leased(app, NativeAction::CreateVault { params }, lease).await
}

/// No submitted credential or native readout becomes a command result.
pub async fn run(app: AppHandle, action: NativeAction) -> Result<NativeResult, ApiError> {
    if matches!(action, NativeAction::ConfirmRecoveryKit) {
        return Err(cancelled());
    }
    let lease = reserve(&app)?;
    run_leased(app, action, lease).await
}

async fn run_leased(
    app: AppHandle,
    action: NativeAction,
    lease: Lease,
) -> Result<NativeResult, ApiError> {
    let token = lease.token;
    let deadline = lease.deadline;
    let entry_kind = match &action {
        NativeAction::CreateVault { .. } | NativeAction::SetNewPassword => EntryKind::NewPassword,
        NativeAction::ChangePassword { .. } => EntryKind::PasswordChange,
        NativeAction::Unlock { recovery: true }
        | NativeAction::InspectBackup { recovery: true }
        | NativeAction::InspectSafetyCopy { recovery: true, .. } => EntryKind::Recovery,
        _ => EntryKind::Password,
    };
    let reveal_mode = match &action {
        NativeAction::ShowWalletField {
            mode: ShowMode::Held,
            ..
        } => RevealMode::Held,
        _ => RevealMode::Timed,
    };
    let begin_app = app.clone();
    let operation = core_work(&app, move |core| {
        check_live(&begin_app, token)?;
        let active = begin_app
            .state::<CoordinatorState>()
            .snapshot(token)
            .ok_or_else(cancelled)?;
        if !active_matches(&active, core) {
            return Err(cancelled());
        }
        let operation = core.begin_native(action)?;
        if !begin_app.state::<CoordinatorState>().publish(
            token,
            operation.id,
            deadline,
            Stage::Entry,
        ) {
            core.cancel_native(operation.id);
            return Err(cancelled());
        }
        Ok(operation)
    })
    .await?;
    let credentials = if operation.requires_credentials {
        let prompt = entry_prompt(&operation, entry_kind);
        let reply = ask(&lease, operation.id, prompt).await?;
        credentials(entry_kind, reply)?
    } else {
        NativeCredentials::None
    };
    let mut operation = operation;
    let mut credentials = credentials;
    loop {
        let prepare_app = app.clone();
        let id = operation.id;
        let job = core_work(&app, move |core| {
            check_live(&prepare_app, token)?;
            core.prepare_native(id, credentials)
        })
        .await?;
        let completion = tauri::async_runtime::spawn_blocking(move || job.run())
            .await
            .map_err(|_| unavailable())?;
        let finish_app = app.clone();
        let outcome = core_work(&app, move |core| {
            check_live(&finish_app, token)?;
            let result = core.finish_native(completion)?;
            let state = finish_app.state::<CoordinatorState>();
            match result {
                NativeResult::Field(value) => {
                    let status = core.app_status();
                    let shown = status.reveal.ok_or_else(cancelled)?;
                    let identity = RevealIdentity {
                        epoch: core.native_operation_epoch(),
                        generation: status.session_generation,
                        note_id: shown.note_id,
                        field: shown.field,
                    };
                    if !state.publish(token, id, deadline, Stage::Reveal(identity)) {
                        core.hide_field();
                        return Err(cancelled());
                    }
                    Ok(Outcome::Reveal(value))
                }
                NativeResult::RecoveryKit(mut kit) => {
                    if let Some(recovery) = kit.recovery_key.take() {
                        let next = match core.begin_native(NativeAction::ConfirmRecoveryKit) {
                            Ok(next) => next,
                            Err(error) => {
                                core.lock();
                                return Err(error);
                            }
                        };
                        if !state.publish(
                            token,
                            next.id,
                            deadline,
                            Stage::RecoveryKit {
                                epoch: core.native_operation_epoch(),
                            },
                        ) {
                            core.lock();
                            return Err(cancelled());
                        }
                        Ok(Outcome::RecoveryKit {
                            recovery,
                            operation: next,
                        })
                    } else {
                        state.stop(token);
                        Ok(Outcome::Return(NativeResult::Unit))
                    }
                }
                result => {
                    state.stop(token);
                    Ok(Outcome::Return(result))
                }
            }
        })
        .await?;
        match outcome {
            Outcome::Return(result) => return Ok(result),
            Outcome::Reveal(value) => {
                reveal(&lease, &operation, value, reveal_mode).await?;
                return Ok(NativeResult::Unit);
            }
            Outcome::RecoveryKit {
                recovery,
                operation: next,
            } => {
                let groups = last_groups(&recovery)?;
                let reply = ask(&lease, next.id, Prompt::RecoveryKit {
                    title: "Save your Recovery Kit".to_owned(),
                    detail: "Write down the Recovery Key and keep it somewhere safe. Retype the final two groups to finish. It stays in this native window.".to_owned(),
                    recovery,
                }).await?;
                if !matches!(reply, Reply::RecoveryKitConfirmed) {
                    return Err(cancelled());
                }
                credentials = NativeCredentials::KitConfirmation(groups);
                operation = next;
            }
        }
    }
}

enum Outcome {
    Return(NativeResult),
    Reveal(Zeroizing<String>),
    RecoveryKit {
        recovery: Zeroizing<String>,
        operation: NativeOperation,
    },
}

async fn core_work<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&mut AppCore) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || commands::with_core(&app, work))
        .await
        .map_err(|_| unavailable())?
}

fn check_live(app: &AppHandle, token: u64) -> Result<(), ApiError> {
    let state = app.state::<CoordinatorState>();
    if state
        .snapshot(token)
        .is_some_and(|active| Instant::now() < active.deadline)
    {
        Ok(())
    } else {
        Err(cancelled())
    }
}

async fn ask(lease: &Lease, id: u64, prompt: Prompt) -> Result<Reply, ApiError> {
    let (tx, rx) = mpsc::channel();
    let app = lease.app.clone();
    let token = lease.token;
    let deadline = lease.deadline;
    lease
        .app
        .run_on_main_thread(move || {
            if !can_present(&app, token, id, deadline) {
                let _ = tx.send(Err(PromptError::Cancelled));
                return;
            }
            let Some(window) = app.get_webview_window("main") else {
                let _ = tx.send(Err(PromptError::Unavailable));
                return;
            };
            let failed = tx.clone();
            if let Err(error) = native_credential::present(
                &window,
                id,
                prompt,
                activity_hook(&app, token, deadline),
                Box::new(move |reply| {
                    let _ = tx.send(reply);
                }),
            ) {
                let _ = failed.send(Err(error));
            }
        })
        .map_err(|_| unavailable())?;
    let reply = receive(rx, lease.deadline).await?.map_err(prompt_error)?;
    check_live(&lease.app, lease.token)?;
    Ok(reply)
}

async fn reveal(
    lease: &Lease,
    operation: &NativeOperation,
    value: Zeroizing<String>,
    mode: RevealMode,
) -> Result<(), ApiError> {
    let (tx, rx) = mpsc::channel();
    let app = lease.app.clone();
    let token = lease.token;
    let id = operation.id;
    let deadline = lease.deadline;
    let detail = action_detail(operation);
    lease
        .app
        .run_on_main_thread(move || {
            if !can_present(&app, token, id, deadline) {
                let _ = tx.send(Err(PromptError::Cancelled));
                return;
            }
            let Some(window) = app.get_webview_window("main") else {
                let _ = tx.send(Err(PromptError::Unavailable));
                return;
            };
            let extend_app = app.clone();
            let failed = tx.clone();
            if let Err(error) = native_credential::present_reveal(
                &window,
                id,
                Reveal {
                    title: "Wallet Hidden Field".to_owned(),
                    detail,
                    value,
                    mode,
                },
                Box::new(move || extend(&extend_app, token, deadline)),
                activity_hook(&app, token, deadline),
                Box::new(move || {
                    let _ = tx.send(Ok(()));
                }),
            ) {
                let _ = failed.send(Err(error));
            }
        })
        .map_err(|_| unavailable())?;
    receive(rx, lease.deadline).await?.map_err(prompt_error)?;
    let app = lease.app.clone();
    core_work(&lease.app, move |core| {
        check_live(&app, token)?;
        core.hide_field();
        app.state::<CoordinatorState>().stop(token);
        Ok(())
    })
    .await
}

async fn receive<T: Send + 'static>(
    rx: mpsc::Receiver<T>,
    deadline: Instant,
) -> Result<T, ApiError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(remaining))
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| cancelled())
}

/// The UI thread must never wait for the core or a KDF worker.
fn can_present(app: &AppHandle, token: u64, id: u64, deadline: Instant) -> bool {
    authorize_callback(
        &app.state::<CoreState>().0,
        &app.state::<CoordinatorState>(),
        token,
        deadline,
        Some(id),
        false,
    ) != Authorization::Denied
}

fn retry_if_live(state: &CoordinatorState, token: u64, deadline: Instant) -> Authorization {
    if state.live(token) && Instant::now() < deadline {
        Authorization::Retry
    } else {
        Authorization::Denied
    }
}

fn authorize_callback(
    core: &Mutex<AppCore>,
    state: &CoordinatorState,
    token: u64,
    deadline: Instant,
    id: Option<u64>,
    genuine_owner_activity: bool,
) -> Authorization {
    let mut core = match core.try_lock() {
        Ok(core) => core,
        Err(TryLockError::WouldBlock) => return retry_if_live(state, token, deadline),
        Err(TryLockError::Poisoned(_)) => return Authorization::Denied,
    };
    authorize_core_callback(
        &mut core,
        state,
        token,
        deadline,
        id,
        genuine_owner_activity,
    )
}

/// Polling and key repeat may check a readout, but only fresh owner input resets Auto-lock.
/// Tick first: cached status can still describe a reveal after its idle/reveal deadline.
fn authorize_core_callback(
    core: &mut AppCore,
    state: &CoordinatorState,
    token: u64,
    deadline: Instant,
    id: Option<u64>,
    genuine_owner_activity: bool,
) -> Authorization {
    core.tick();
    let active = match state.try_snapshot(token) {
        Snapshot::Active(active) => active,
        Snapshot::Retry => return retry_if_live(state, token, deadline),
        Snapshot::Denied => return Authorization::Denied,
    };
    if !active_authorized(&active, core, state, deadline, id) {
        return Authorization::Denied;
    }
    if genuine_owner_activity {
        core.activity();
    }
    if active_authorized(&active, core, state, deadline, id) {
        Authorization::Allowed
    } else {
        Authorization::Denied
    }
}

fn active_authorized(
    active: &Active,
    core: &AppCore,
    state: &CoordinatorState,
    deadline: Instant,
    id: Option<u64>,
) -> bool {
    state.live(active.token)
        && id.is_none_or(|id| id == active.id)
        && Instant::now() < deadline
        && Instant::now() < active.deadline
        && active_matches(active, core)
}

fn active_matches(active: &Active, core: &AppCore) -> bool {
    let status = core.app_status();
    if status.sharing_guard.up {
        return false;
    }
    match &active.stage {
        Stage::Calibration {
            epoch,
            generation,
            phase,
        } => {
            *epoch == core.native_operation_epoch()
                && *generation == status.session_generation
                && *phase == status.phase
        }
        Stage::Entry | Stage::RecoveryKit { .. } => {
            active.id == 0 || core.native_pending_id() == Some(active.id)
        }
        Stage::Reveal(identity) => identity.matches(&status, core.native_operation_epoch()),
    }
}

fn activity_hook(app: &AppHandle, token: u64, deadline: Instant) -> native_credential::Activity {
    let app = app.clone();
    Box::new(move |genuine_owner_activity| {
        authorize_callback(
            &app.state::<CoreState>().0,
            &app.state::<CoordinatorState>(),
            token,
            deadline,
            None,
            genuine_owner_activity,
        )
    })
}

fn extend(app: &AppHandle, token: u64, deadline: Instant) -> Authorization {
    extend_callback(
        &app.state::<CoreState>().0,
        &app.state::<CoordinatorState>(),
        token,
        deadline,
        |core| {
            let _ = app.emit(STATUS_EVENT, commands::status(core));
        },
    )
}

fn extend_callback(
    core: &Mutex<AppCore>,
    state: &CoordinatorState,
    token: u64,
    deadline: Instant,
    on_extended: impl FnOnce(&AppCore),
) -> Authorization {
    let mut core = match core.try_lock() {
        Ok(core) => core,
        Err(TryLockError::WouldBlock) => return retry_if_live(state, token, deadline),
        Err(TryLockError::Poisoned(_)) => return Authorization::Denied,
    };
    core.tick();
    if !core
        .app_status()
        .reveal
        .is_some_and(|shown| shown.can_extend)
    {
        return Authorization::Denied;
    }
    let active = match state.try_snapshot(token) {
        Snapshot::Active(active) => active,
        Snapshot::Retry => return retry_if_live(state, token, deadline),
        Snapshot::Denied => return Authorization::Denied,
    };
    if !matches!(&active.stage, Stage::Reveal(_))
        || !active_authorized(&active, &core, state, deadline, None)
        || core.extend_reveal().is_err()
        || !active_authorized(&active, &core, state, deadline, None)
    {
        return Authorization::Denied;
    }
    on_extended(&core);
    Authorization::Allowed
}

/// Called after tick/guard updates while the caller already holds CoreState.
pub fn sync(app: &AppHandle, core: &mut AppCore) {
    let state = app.state::<CoordinatorState>();
    let token = state.current.load(Ordering::SeqCst);
    let Some(active) = state.snapshot(token) else {
        return;
    };
    if Instant::now() < active.deadline && active_matches(&active, core) {
        return;
    }
    if let Some(active) = state.stop(token) {
        cleanup_core(core, &active, state.idle());
        close_panel(app, active.id);
    }
}

/// Invalidate callbacks immediately; core cleanup and UI dismissal never wait on the main thread.
pub fn cancel(app: &AppHandle) {
    let token = app
        .state::<CoordinatorState>()
        .current
        .load(Ordering::SeqCst);
    if token != 0 {
        cancel_token(app, token);
    }
}

fn cancel_token(app: &AppHandle, token: u64) {
    let Some(active) = app.state::<CoordinatorState>().stop(token) else {
        return;
    };
    close_panel(app, active.id);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        commands::with_core(&app, |core| {
            cleanup_core(core, &active, app.state::<CoordinatorState>().idle());
        });
    });
}

fn cleanup_core(core: &mut AppCore, active: &Active, coordinator_idle: bool) {
    if let Stage::RecoveryKit { epoch } = active.stage
        && (core.native_pending_id() == Some(active.id) || core.native_operation_epoch() == epoch)
        && core.app_status().phase == "confirm_recovery_kit"
    {
        // A later reservation does not own this still-unsaved candidate.
        core.lock();
        return;
    }
    if active.id != 0 {
        core.cancel_native(active.id);
    }
    if !coordinator_idle {
        return;
    }
    match &active.stage {
        Stage::Reveal(identity) if identity.same_field(&core.app_status()) => core.hide_field(),
        Stage::RecoveryKit { .. } if core.app_status().phase == "confirm_recovery_kit" => {
            core.lock()
        }
        _ => (),
    }
}

fn close_panel(app: &AppHandle, id: u64) {
    if id != 0 {
        let _ = app.run_on_main_thread(move || {
            native_credential::cancel(id);
        });
    }
}

fn entry_prompt(operation: &NativeOperation, kind: EntryKind) -> Prompt {
    let title = match operation.action {
        "create_vault" => "Create your Vault",
        "unlock" => "Unlock your Vault",
        "set_new_password" => "Set a new Master Password",
        "create_note" => "Create Wallet Note",
        "update_note" => "Save Wallet Note",
        "trash_note" => "Move Wallet Note to Trash",
        "delete_forever" => "Delete Wallet Note forever",
        "empty_trash" => "Empty Trash containing Wallet Notes",
        "show_wallet_field" => "Show Wallet Hidden Field",
        "show_and_copy_wallet_field" => "Show and copy Wallet Hidden Field",
        "copy_field" => "Copy Wallet Hidden Field",
        "inspect_backup" => "Open a Backup",
        "inspect_safety_copy" => "Open a Safety Copy",
        "change_password" => "Change Master Password",
        "rotate_keys" => "Rotate Vault keys",
        "set_settings" => "Confirm less strict Auto-lock",
        "enroll_quick_unlock" => "Enable Touch ID for this Vault",
        _ => "Confirm encrypted-note action",
    }
    .to_owned();
    let detail = action_detail(operation);
    match kind {
        EntryKind::NewPassword | EntryKind::PasswordChange => Prompt::NewPassword {
            title,
            detail,
            current_password: matches!(kind, EntryKind::PasswordChange),
        },
        EntryKind::Password | EntryKind::Recovery => Prompt::Credential {
            title,
            detail,
            label: if matches!(kind, EntryKind::Recovery) {
                "Recovery Key"
            } else {
                "Master Password"
            }
            .to_owned(),
        },
    }
}

fn action_detail(operation: &NativeOperation) -> String {
    let mut detail =
        "This approval applies only to this action. Credentials stay in native controls."
            .to_owned();
    if let Some(target) = &operation.target {
        let title: String = target
            .chars()
            .filter(|c| !c.is_control())
            .take(120)
            .collect();
        detail.push_str(&format!("\nNote: {title}"));
    }
    if let Some(kind) = operation.kind {
        detail.push_str(match kind {
            Kind::SeedPhrase => "\nKind: Seed Phrase",
            Kind::PrivateKey => "\nKind: Private Key",
            Kind::Login => "\nKind: Login",
            Kind::ApiKey => "\nKind: API Key",
            Kind::Text => "\nKind: Text",
        });
    }
    if let Some(field) = &operation.field {
        let field = match field.as_str() {
            "words" => "Seed Phrase",
            "passphrase" => "Wallet passphrase",
            "key" => "Private Key",
            _ => "Hidden Field",
        };
        detail.push_str(&format!("\nField: {field}"));
    }
    if operation.kind == Some(Kind::SeedPhrase)
        && matches!(
            operation.action,
            "copy_field" | "show_and_copy_wallet_field"
        )
    {
        detail.push_str("\nCopying a Seed Phrase can expose your wallet through clipboard history or another app. Continue only if you need to paste it. encrypted-note clears its own copy after 30 seconds if unchanged; clipboard managers and receiving apps may retain it.");
    }
    detail
}

fn credentials(kind: EntryKind, reply: Reply) -> Result<NativeCredentials, ApiError> {
    Ok(match (kind, reply) {
        (EntryKind::Password, Reply::Credential(value)) => NativeCredentials::Password(value),
        (EntryKind::Recovery, Reply::Credential(value)) => NativeCredentials::RecoveryKey(value),
        (EntryKind::NewPassword, Reply::NewPassword { current: None, new }) => {
            NativeCredentials::NewPassword {
                confirmation: new.clone(),
                password: new,
            }
        }
        (
            EntryKind::PasswordChange,
            Reply::NewPassword {
                current: Some(current),
                new,
            },
        ) => NativeCredentials::PasswordChange {
            current,
            confirmation: new.clone(),
            password: new,
        },
        _ => return Err(cancelled()),
    })
}

fn last_groups(recovery: &str) -> Result<Zeroizing<String>, ApiError> {
    let mut groups = recovery.rsplit('-');
    let last = groups.next().ok_or_else(unavailable)?;
    let previous = groups.next().ok_or_else(unavailable)?;
    if last.len() != 4 || previous.len() != 4 || !last.is_ascii() || !previous.is_ascii() {
        return Err(unavailable());
    }
    let mut confirmation = Zeroizing::new(String::with_capacity(8));
    confirmation.push_str(previous);
    confirmation.push_str(last);
    Ok(confirmation)
}

fn prompt_error(error: PromptError) -> ApiError {
    match error {
        PromptError::Cancelled => cancelled(),
        PromptError::Busy => busy(),
        PromptError::Unavailable => unavailable(),
    }
}
fn cancelled() -> ApiError {
    ApiError {
        code: "cancelled",
        message: "The native action was cancelled. Please try again.".to_owned(),
    }
}
fn busy() -> ApiError {
    ApiError {
        code: "invalid_input",
        message: "Finish or cancel the current native action first.".to_owned(),
    }
}
fn unavailable() -> ApiError {
    ApiError {
        code: "internal",
        message: "The native action could not finish. Please try again.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_actions_and_late_callbacks_cannot_reuse_a_lease() {
        let state = CoordinatorState::default();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let first = state.reserve(deadline).unwrap();
        assert!(state.reserve(deadline).is_err());
        assert!(state.stop(first).is_some());
        assert!(state.idle());
        let second = state.reserve(deadline).unwrap();
        assert!(!state.publish(first, 9, deadline, Stage::Entry));
        assert!(state.stop(first).is_none());
        assert!(state.live(second));
        assert!(state.publish(second, 10, deadline, Stage::RecoveryKit { epoch: 0 }));
        assert_eq!(state.snapshot(second).unwrap().id, 10);
    }

    #[test]
    fn stop_keeps_ownership_until_panel_metadata_can_be_removed() {
        use std::sync::Arc;
        let state = Arc::new(CoordinatorState::default());
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let first = state.reserve(deadline).unwrap();
        assert!(state.publish(first, 41, deadline, Stage::RecoveryKit { epoch: 0 }));
        let metadata = state.active.lock().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker_state = state.clone();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(worker_state.stop(first)).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        assert_eq!(state.current.load(Ordering::SeqCst), first);
        drop(metadata);
        assert_eq!(
            done_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap()
                .id,
            41
        );
        worker.join().unwrap();
        let second = state.reserve(deadline).unwrap();
        assert!(state.live(second));
        assert!(state.publish(second, 42, deadline, Stage::Entry));
        assert_eq!(state.snapshot(second).unwrap().id, 42);
    }

    struct FakeClock(std::sync::Arc<AtomicU64>);
    impl crate::core::Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
        fn unix_secs(&self) -> i64 {
            1_790_000_000
        }
    }
    struct FakeClipboard;
    impl crate::core::Clipboard for FakeClipboard {
        fn write(&mut self, _: &str) -> Result<u64, ApiError> {
            Ok(1)
        }
        fn clear_if(&mut self, _: u64) -> bool {
            true
        }
    }
    fn fake_core() -> (AppCore, tempfile::TempDir) {
        let (core, temp, _) = fake_core_with_clock();
        (core, temp)
    }
    fn fake_core_with_clock() -> (AppCore, tempfile::TempDir, std::sync::Arc<AtomicU64>) {
        let temp = tempfile::tempdir().unwrap();
        let now = std::sync::Arc::new(AtomicU64::new(1_000));
        let core = AppCore::new(crate::core::Config {
            data_dir: temp.path().join("Vault.noindex"),
            home_dir: temp.path().to_owned(),
            limits: vault_format::Limits::TESTING,
            clock: Box::new(FakeClock(now.clone())),
            clipboard: Box::new(FakeClipboard),
            activity_anywhere_supported: false,
        })
        .unwrap();
        (core, temp, now)
    }
    fn setup_fake_vault(core: &mut AppCore) -> Zeroizing<String> {
        core.create_vault(
            Zeroizing::new("fake-lantern-quartz-mosaic-velvet-orbit".to_owned()),
            vault_format::KdfParams::TESTING,
        )
        .unwrap()
        .recovery_key
        .unwrap()
    }

    #[test]
    fn no_vault_context_changes_cancel_calibration_even_without_a_phase_change() {
        let (mut core, _temp) = fake_core();
        let status = core.app_status();
        let active = Active {
            token: 1,
            id: 0,
            deadline: Instant::now() + OPERATION_TIMEOUT,
            stage: Stage::Calibration {
                epoch: core.native_operation_epoch(),
                generation: status.session_generation,
                phase: status.phase,
            },
        };
        assert!(active_matches(&active, &core));
        core.hide_field();
        assert_eq!(core.app_status().phase, status.phase);
        assert_eq!(
            core.app_status().session_generation,
            status.session_generation
        );
        assert!(!active_matches(&active, &core));
    }

    #[test]
    fn a_later_reservation_cannot_orphan_an_owned_recovery_candidate() {
        let (mut core, _temp) = fake_core();
        let _fake_key = setup_fake_vault(&mut core);
        let operation = core.begin_native(NativeAction::ConfirmRecoveryKit).unwrap();
        let active = Active {
            token: 1,
            id: operation.id,
            deadline: Instant::now() + OPERATION_TIMEOUT,
            stage: Stage::RecoveryKit {
                epoch: core.native_operation_epoch(),
            },
        };
        cleanup_core(&mut core, &active, false);
        assert_eq!(core.app_status().phase, "no_vault");
        assert!(core.native_pending_id().is_none());
    }

    #[test]
    fn a_document_mutation_invalidates_and_clears_the_native_readout() {
        use std::collections::BTreeMap;
        let (mut core, _temp) = fake_core();
        let fake_key = setup_fake_vault(&mut core);
        core.confirm_recovery_kit(&last_groups(&fake_key).unwrap())
            .unwrap();
        let id = core
            .create_note(
                crate::core::NoteInput {
                    kind: Kind::Text,
                    title: "Fake note".to_owned(),
                    visible: BTreeMap::new(),
                    hidden: BTreeMap::from([(
                        "body".to_owned(),
                        Zeroizing::new("fake-body-only".to_owned()),
                    )]),
                },
                None,
            )
            .unwrap();
        let _fake_value = core.show_field(&id, "body").unwrap();
        let identity = RevealIdentity {
            epoch: core.native_operation_epoch(),
            generation: core.app_status().session_generation,
            note_id: id.clone(),
            field: "body".to_owned(),
        };
        assert!(identity.matches(&core.app_status(), core.native_operation_epoch()));
        core.set_favorite(&id, true).unwrap();
        assert!(identity.same_field(&core.app_status()));
        assert!(!identity.matches(&core.app_status(), core.native_operation_epoch()));
        cleanup_core(
            &mut core,
            &Active {
                token: 1,
                id: 0,
                deadline: Instant::now() + OPERATION_TIMEOUT,
                stage: Stage::Reveal(identity),
            },
            true,
        );
        assert!(core.app_status().reveal.is_none());
    }

    fn authorized_for_test(
        core: &mut AppCore,
        state: &CoordinatorState,
        token: u64,
        id: Option<u64>,
        genuine_owner_activity: bool,
    ) -> bool {
        let Some(active) = state.snapshot(token) else {
            return false;
        };
        authorize_core_callback(
            core,
            state,
            token,
            active.deadline,
            id,
            genuine_owner_activity,
        ) == Authorization::Allowed
    }

    fn fake_wallet_readout(core: &mut AppCore, state: &CoordinatorState) -> u64 {
        fake_wallet_readout_mode(core, state, ShowMode::Held)
    }

    fn fake_wallet_readout_mode(
        core: &mut AppCore,
        state: &CoordinatorState,
        mode: ShowMode,
    ) -> u64 {
        use std::collections::BTreeMap;
        let fake_key = setup_fake_vault(core);
        core.confirm_recovery_kit(&last_groups(&fake_key).unwrap())
            .unwrap();
        // Keep the idle deadline earlier than the independent two-minute held-reveal backstop.
        core.set_settings(
            serde_json::from_value(serde_json::json!({
                "idle_minutes": 1,
                "lock_on_app_switch": false
            }))
            .unwrap(),
        )
        .unwrap();
        let id = core
            .create_note(
                crate::core::NoteInput {
                    kind: Kind::PrivateKey,
                    title: "Fake Wallet only".to_owned(),
                    visible: BTreeMap::new(),
                    hidden: BTreeMap::from([(
                        "key".to_owned(),
                        Zeroizing::new("fake-private-key-only".to_owned()),
                    )]),
                },
                Some(Zeroizing::new(
                    "fake-lantern-quartz-mosaic-velvet-orbit".to_owned(),
                )),
            )
            .unwrap();
        let _fake_value = core
            .show_wallet_field(
                &id,
                "key",
                Zeroizing::new("fake-lantern-quartz-mosaic-velvet-orbit".to_owned()),
                mode,
            )
            .unwrap();
        let status = core.app_status();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let token = state.reserve(deadline).unwrap();
        assert!(state.publish(
            token,
            71,
            deadline,
            Stage::Reveal(RevealIdentity {
                epoch: core.native_operation_epoch(),
                generation: status.session_generation,
                note_id: id,
                field: "key".to_owned(),
            }),
        ));
        token
    }

    #[test]
    fn fresh_input_cannot_repaint_a_wallet_readout_after_the_idle_deadline() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let idle_ms = core.app_status().lock_in_ms.unwrap();
        now.fetch_add(idle_ms, Ordering::SeqCst);
        // Simulate a native event arriving before the periodic background tick.
        assert!(active_matches(&state.snapshot(token).unwrap(), &core));
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert_eq!(core.app_status().phase, "locked");
        assert!(core.app_status().reveal.is_none());
    }

    #[test]
    fn polling_and_key_repeat_never_extend_a_held_wallet_readouts_idle_time() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let idle_ms = core.app_status().lock_in_ms.unwrap();
        now.fetch_add(idle_ms - 1_000, Ordering::SeqCst);
        assert!(authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            false
        ));
        assert_eq!(core.app_status().lock_in_ms, Some(1_000));
        now.fetch_add(1_000, Ordering::SeqCst);
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            false
        ));
        assert_eq!(core.app_status().phase, "locked");
        assert!(core.app_status().reveal.is_none());
    }

    #[test]
    fn fresh_owner_input_extends_idle_time_only_for_the_current_live_readout() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let idle_ms = core.app_status().lock_in_ms.unwrap();
        now.fetch_add(idle_ms - 1_000, Ordering::SeqCst);
        assert!(authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert_eq!(core.app_status().lock_in_ms, Some(idle_ms));
        state.stop(token);
        now.fetch_add(1_000, Ordering::SeqCst);
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert_eq!(core.app_status().lock_in_ms, Some(idle_ms - 1_000));
    }

    #[test]
    fn held_backstop_hides_the_readout_even_when_owner_activity_keeps_the_vault_open() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        for _ in 0..2 {
            now.fetch_add(50_000, Ordering::SeqCst);
            assert!(authorized_for_test(
                &mut core,
                &state,
                token,
                Some(71),
                true
            ));
        }
        now.fetch_add(20_000, Ordering::SeqCst);
        assert!(active_matches(&state.snapshot(token).unwrap(), &core));
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert_eq!(core.app_status().phase, "unlocked");
        assert!(core.app_status().reveal.is_none());
        assert_eq!(core.app_status().lock_in_ms, Some(40_000));
    }

    #[test]
    fn temporary_core_contention_retries_without_counting_owner_activity() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let deadline = state.snapshot(token).unwrap().deadline;
        now.fetch_add(1_000, Ordering::SeqCst);
        let core = Mutex::new(core);
        let held = core.lock().unwrap();
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, Some(71), true),
            Authorization::Retry
        );
        assert_eq!(held.app_status().lock_in_ms, Some(59_000));
        assert_eq!(
            authorize_callback(&core, &state, token, Instant::now(), Some(71), true),
            Authorization::Denied
        );
        drop(held);
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, Some(71), false),
            Authorization::Allowed
        );
        assert_eq!(core.lock().unwrap().app_status().lock_in_ms, Some(59_000));
    }

    #[test]
    fn temporary_metadata_contention_retries_but_a_cancelled_lease_is_denied() {
        let (core, _temp) = fake_core();
        let core = Mutex::new(core);
        let state = CoordinatorState::default();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let token = state.reserve(deadline).unwrap();
        let metadata = state.active.lock().unwrap();
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, None, true),
            Authorization::Retry
        );
        assert_eq!(
            authorize_callback(&core, &state, token, Instant::now(), None, true),
            Authorization::Denied
        );
        // Cancellation remains observable even when its metadata cleanup has not completed.
        state.current.store(0, Ordering::SeqCst);
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, None, true),
            Authorization::Denied
        );
        drop(metadata);
    }

    #[test]
    fn poisoned_core_or_metadata_is_denied_instead_of_retried() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let (core, _temp) = fake_core();
        let core = Mutex::new(core);
        let state = CoordinatorState::default();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let token = state.reserve(deadline).unwrap();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _held = core.lock().unwrap();
                panic!("fictional core poisoning for a regression test");
            }))
            .is_err()
        );
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, None, false),
            Authorization::Denied
        );
        let (core, _other_temp) = fake_core();
        let core = Mutex::new(core);
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _held = state.active.lock().unwrap();
                panic!("fictional metadata poisoning for a regression test");
            }))
            .is_err()
        );
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, None, false),
            Authorization::Denied
        );
    }

    #[test]
    fn core_contention_does_not_close_or_consume_a_native_extension() {
        use std::cell::Cell;
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout_mode(&mut core, &state, ShowMode::Accessible);
        let deadline = state.snapshot(token).unwrap().deadline;
        let notifications = Cell::new(0);
        now.fetch_add(5_000, Ordering::SeqCst);
        let core = Mutex::new(core);
        let held = core.lock().unwrap();
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| notifications.set(1)),
            Authorization::Retry
        );
        let shown = held.app_status().reveal.unwrap();
        assert_eq!(shown.extensions_left, Some(3));
        assert_eq!(shown.expires_in_ms, Some(15_000));
        assert_eq!(notifications.get(), 0);
        assert_eq!(
            extend_callback(&core, &state, token, Instant::now(), |_| notifications
                .set(1)),
            Authorization::Denied
        );
        drop(held);
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| notifications.set(1)),
            Authorization::Allowed
        );
        let shown = core.lock().unwrap().app_status().reveal.unwrap();
        assert_eq!(shown.extensions_left, Some(2));
        assert_eq!(shown.expires_in_ms, Some(20_000));
        assert_eq!(notifications.get(), 1);
    }

    #[test]
    fn metadata_contention_retries_an_extension_but_never_revives_an_expired_readout() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout_mode(&mut core, &state, ShowMode::Accessible);
        let deadline = state.snapshot(token).unwrap().deadline;
        let core = Mutex::new(core);
        let metadata = state.active.lock().unwrap();
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| panic!("not extended")),
            Authorization::Retry
        );
        assert_eq!(
            core.lock()
                .unwrap()
                .app_status()
                .reveal
                .unwrap()
                .extensions_left,
            Some(3)
        );
        now.fetch_add(20_000, Ordering::SeqCst);
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| panic!("not extended")),
            Authorization::Denied
        );
        assert!(core.lock().unwrap().app_status().reveal.is_none());
        drop(metadata);
    }

    #[test]
    fn native_extension_limit_remains_a_denial_after_three_successful_extensions() {
        let (mut core, _temp, _now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout_mode(&mut core, &state, ShowMode::Accessible);
        let deadline = state.snapshot(token).unwrap().deadline;
        let core = Mutex::new(core);
        for _ in 0..3 {
            assert_eq!(
                extend_callback(&core, &state, token, deadline, |_| ()),
                Authorization::Allowed
            );
        }
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| panic!("not extended")),
            Authorization::Denied
        );
        assert_eq!(
            core.lock()
                .unwrap()
                .app_status()
                .reveal
                .unwrap()
                .extensions_left,
            Some(0)
        );
    }

    #[test]
    fn touch_id_enrollment_prompt_names_the_action() {
        let operation = NativeOperation {
            id: 1,
            action: "enroll_quick_unlock",
            target: None,
            kind: None,
            field: None,
            requires_credentials: true,
        };
        let Prompt::Credential { title, label, .. } = entry_prompt(&operation, EntryKind::Password)
        else {
            panic!("Touch ID enrollment requires native credential entry");
        };
        assert_eq!(title, "Enable Touch ID for this Vault");
        assert_eq!(label, "Master Password");
    }

    #[test]
    fn seed_copy_warning_is_owned_by_the_native_prompt() {
        let mut operation = NativeOperation {
            id: 1,
            action: "copy_field",
            target: Some("Fake Wallet".to_owned()),
            kind: Some(Kind::SeedPhrase),
            field: Some("words".to_owned()),
            requires_credentials: true,
        };
        assert!(action_detail(&operation).contains("clipboard history or another app"));
        operation.action = "show_and_copy_wallet_field";
        assert!(
            action_detail(&operation)
                .contains("clipboard managers and receiving apps may retain it")
        );
        operation.action = "show_wallet_field";
        assert!(!action_detail(&operation).contains("clipboard history"));
    }

    #[test]
    fn kit_confirmation_is_derived_only_inside_rust() {
        let fake = "FAKE-FAKE-FAKE-FAKE-FAKE-ABCD-EF01";
        assert_eq!(last_groups(fake).unwrap().as_str(), "ABCDEF01");
        assert!(last_groups("fake").is_err());
        assert!(last_groups("FAKE-LONGER").is_err());
    }

    #[test]
    fn credential_reply_shape_is_bound_to_the_validated_action() {
        assert!(
            credentials(
                EntryKind::PasswordChange,
                Reply::Credential(Zeroizing::new("fake".into()))
            )
            .is_err()
        );
        assert!(credentials(EntryKind::Password, Reply::RecoveryKitConfirmed).is_err());
        assert!(matches!(
            credentials(
                EntryKind::Recovery,
                Reply::Credential(Zeroizing::new("fake-recovery".into()))
            ),
            Ok(NativeCredentials::RecoveryKey(_))
        ));
    }
}
