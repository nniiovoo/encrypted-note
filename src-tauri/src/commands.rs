//! The app bridge: one thin `#[tauri::command]` per function in `src/api.ts`, each a call into
//! [`AppCore`] followed by a "status-changed" event (read-only commands skip the event).
//!
//! No command runs on the main thread: Tauri runs plain `fn` commands there unless they are
//! marked `(async)`, and waiting for the core while an unlock runs Argon2id would freeze the
//! window. Commands that run Argon2id (about a second) or wait for a file dialog are `async fn`
//! and do that work on the blocking pool.
//!
//! The webview may call exactly these commands: `build.rs` lists them in the app manifest and
//! `capabilities/main.json` grants each one (plus listening to events and dragging the window).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError, mpsc};
use std::time::Duration;

use crate::core::native::{NativeAction, NativeResult};
use notes::seed::SeedCheck;
use notes::{Filter, NoteSummary, NoteView};
use serde::Deserialize;
use session::OsEvent;
use tauri::{AppHandle, Emitter, Manager};
use zeroize::Zeroizing;

use crate::agent_route::{self, AgentRoute, RouteError};
use crate::companion::{Companion, CompanionView};
use crate::core::agent::AgentPendingView;
use crate::core::{
    ApiError, AppCore, AppStatus, BackupWritten, NoteEditInput, NoteInput, PickedFile, SafetyCopy,
    SettingsInput, SettingsView, ShowMode, VaultPreview,
};

pub const STATUS_EVENT: &str = "status-changed";

/// Managed state: the one `AppCore`.
pub struct CoreState(pub Mutex<AppCore>);

impl CoreState {
    fn lock(&self, focused: &AtomicBool) -> MutexGuard<'_, AppCore> {
        let mut core = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        // A queued Show or Copy can run before the focus-event worker.
        core.set_window_in_front(focused.load(Ordering::SeqCst));
        core
    }
}

/// True while a Backup file dialog is open. The dialog takes focus from our window, which must
/// not count as switching to another app (that could lock the Vault mid-backup).
pub static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);
static STATUS_REVISION: AtomicU64 = AtomicU64::new(0);

pub fn status(core: &AppCore) -> AppStatus {
    let mut status = core.app_status();
    status.revision = STATUS_REVISION
        .fetch_add(1, Ordering::Relaxed)
        .saturating_add(1);
    status
}

pub fn core(app: &AppHandle) -> MutexGuard<'_, AppCore> {
    app.state::<CoreState>()
        .inner()
        .lock(&crate::WINDOW_FOCUSED)
}

/// Run `f` on the core, then tell the screens the new status.
pub fn with_core<T>(app: &AppHandle, f: impl FnOnce(&mut AppCore) -> T) -> T {
    let mut core = core(app);
    let out = f(&mut core);
    crate::native_coordinator::sync(app, &mut core);
    crate::quick_runtime::sync(app, &mut core);
    let _ = app.emit(STATUS_EVENT, status(&core));
    out
}

/// Disk-only work waits off the UI thread; credential derivation uses the native coordinator.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&mut AppCore) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tauri::async_runtime::spawn_blocking(move || with_core(&app, f))
        .await
        .map_err(|_| internal())?
}

/// Its own text, not the core's: a panic can land here after a save has already happened.
fn internal() -> ApiError {
    ApiError {
        code: "internal",
        message: "Something went wrong inside encrypted-note. Your last action may not have finished. Check it before you try again. If it keeps happening, restart the app.".to_owned(),
    }
}

pub fn os_event(app: &AppHandle, event: OsEvent) {
    if !matches!(event, OsEvent::FocusGained) {
        crate::native_coordinator::interrupt(app);
        crate::quick_runtime::cancel(app);
    }
    with_core(app, |core| {
        core.os_event(event);
        // A focus event that waited for the core may be older than one applied before it.
        if matches!(event, OsEvent::FocusLost | OsEvent::FocusGained) {
            core.set_window_in_front(crate::WINDOW_FOCUSED.load(Ordering::SeqCst));
        }
    });
}

// ---------- status & events ----------

#[tauri::command(async)]
pub fn app_status(app: AppHandle) -> AppStatus {
    status(&core(&app))
}

#[tauri::command(async)]
pub fn activity(app: AppHandle) {
    core(&app).activity();
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    Password,
    Recovery,
}
impl CredentialKind {
    fn is_recovery(&self) -> bool {
        matches!(self, Self::Recovery)
    }
}

/// Credentials are deliberately absent; unknown legacy password input is rejected.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsRequest {
    #[serde(default)]
    session_generation: Option<u64>,
    idle_minutes: u32,
    lock_on_app_switch: bool,
    #[serde(default)]
    clipboard_clear_seconds: Option<u32>,
    #[serde(default)]
    count_activity_anywhere: Option<bool>,
}
impl From<SettingsRequest> for SettingsInput {
    fn from(value: SettingsRequest) -> Self {
        Self {
            master_password: None,
            session_generation: value.session_generation,
            idle_minutes: value.idle_minutes,
            lock_on_app_switch: value.lock_on_app_switch,
            clipboard_clear_seconds: value.clipboard_clear_seconds,
            count_activity_anywhere: value.count_activity_anywhere,
        }
    }
}
async fn native_unit(app: AppHandle, action: NativeAction) -> Result<(), ApiError> {
    match crate::native_coordinator::run(app, action).await? {
        NativeResult::Unit => Ok(()),
        _ => Err(internal()),
    }
}
#[tauri::command(async)]
pub fn cancel_native_operation(app: AppHandle) {
    crate::native_coordinator::cancel(&app);
    crate::quick_runtime::cancel(&app);
}

#[tauri::command]
pub async fn enable_quick_unlock(app: AppHandle) -> Result<(), ApiError> {
    crate::quick_runtime::enable(app).await
}
#[tauri::command]
pub async fn disable_quick_unlock(app: AppHandle) -> Result<(), ApiError> {
    crate::quick_runtime::disable(app).await
}
#[tauri::command]
pub async fn quick_unlock(app: AppHandle) -> Result<(), ApiError> {
    crate::quick_runtime::unlock(app).await
}

// ---------- setup, unlock, lock ----------

#[tauri::command]
pub async fn create_vault(app: AppHandle) -> Result<(), ApiError> {
    match crate::native_coordinator::create(app).await? {
        NativeResult::Unit => Ok(()),
        _ => Err(internal()),
    }
}

#[tauri::command]
pub async fn unlock(app: AppHandle, kind: CredentialKind) -> Result<(), ApiError> {
    native_unit(
        app,
        NativeAction::Unlock {
            recovery: kind.is_recovery(),
        },
    )
    .await
}

#[tauri::command]
pub async fn set_new_password(app: AppHandle) -> Result<(), ApiError> {
    native_unit(app, NativeAction::SetNewPassword).await
}

#[tauri::command(async)]
pub fn lock(app: AppHandle) {
    crate::native_coordinator::cancel(&app);
    crate::quick_runtime::cancel(&app);
    with_core(&app, AppCore::lock);
}

// ---------- Notes ----------

#[tauri::command(async)]
pub fn list_notes(
    app: AppHandle,
    filter: Filter,
    query: String,
) -> Result<Vec<NoteSummary>, ApiError> {
    with_core(&app, |core| core.list_notes(filter, &query))
}

#[tauri::command(async)]
pub fn get_note(app: AppHandle, id: String) -> Result<NoteView, ApiError> {
    with_core(&app, |core| core.get_note(&id))
}

#[tauri::command]
pub async fn create_note(app: AppHandle, input: NoteInput) -> Result<String, ApiError> {
    match crate::native_coordinator::run(app, NativeAction::CreateNote(input)).await? {
        NativeResult::NoteId(id) => Ok(id),
        _ => Err(internal()),
    }
}

#[tauri::command]
pub async fn update_note(app: AppHandle, id: String, input: NoteEditInput) -> Result<(), ApiError> {
    native_unit(app, NativeAction::UpdateNote { id, input }).await
}

#[tauri::command]
pub async fn trash_note(app: AppHandle, id: String) -> Result<(), ApiError> {
    native_unit(app, NativeAction::TrashNote { id }).await
}

#[tauri::command(async)]
pub fn restore_note(app: AppHandle, id: String) -> Result<(), ApiError> {
    with_core(&app, |core| core.restore_note(&id))
}

#[tauri::command]
pub async fn delete_forever(app: AppHandle, id: String) -> Result<(), ApiError> {
    native_unit(app, NativeAction::DeleteForever { id }).await
}

#[tauri::command]
pub async fn empty_trash(app: AppHandle) -> Result<usize, ApiError> {
    match crate::native_coordinator::run(app, NativeAction::EmptyTrash).await? {
        NativeResult::Count(count) => Ok(count),
        _ => Err(internal()),
    }
}

#[tauri::command(async)]
pub fn set_favorite(app: AppHandle, id: String, favorite: bool) -> Result<(), ApiError> {
    with_core(&app, |core| core.set_favorite(&id, favorite))
}

#[tauri::command(async)]
pub fn append_text(app: AppHandle, id: String, text: Zeroizing<String>) -> Result<(), ApiError> {
    with_core(&app, |core| core.append_text(&id, text))
}

// ---------- agent requests (ADR-0012) ----------
//
// The owner's side only: what the screens show for an agent's pending request, and completing
// or cancelling it. The agent's own requests reach the core through the local route, never
// through these commands. Every completion goes through the native coordinator, which asks for
// the Master Password where the request needs it.

/// Read-only (no status event): the screens call it when an `agent-request` event arrives.
#[tauri::command(async)]
pub fn agent_pending(app: AppHandle) -> Option<AgentPendingView> {
    core(&app).agent_pending()
}

/// The owner typed the value the agent asked to store and chose Save. Returns the new Note's id.
#[tauri::command]
pub async fn agent_complete_store(
    app: AppHandle,
    request_id: String,
    input: NoteInput,
) -> Result<String, ApiError> {
    match crate::native_coordinator::run(app, NativeAction::AgentStore { request_id, input })
        .await?
    {
        NativeResult::NoteId(id) => Ok(id),
        _ => Err(internal()),
    }
}

/// The owner allowed the agent's edit in the WebView's confirm.
#[tauri::command]
pub async fn agent_confirm_edit(app: AppHandle, request_id: String) -> Result<(), ApiError> {
    native_unit(app, NativeAction::AgentEdit { request_id }).await
}

/// The owner typed the new value the agent asked for. Hidden Fields only.
#[tauri::command]
pub async fn agent_complete_replace(
    app: AppHandle,
    request_id: String,
    input: NoteEditInput,
) -> Result<(), ApiError> {
    native_unit(app, NativeAction::AgentReplace { request_id, input }).await
}

/// The owner allowed the agent's delete; the Master Password panel follows for every Kind.
#[tauri::command]
pub async fn agent_confirm_delete(app: AppHandle, request_id: String) -> Result<(), ApiError> {
    native_unit(app, NativeAction::AgentTrash { request_id }).await
}

#[tauri::command(async)]
pub fn agent_cancel_request(app: AppHandle, request_id: String) {
    with_core(&app, |core| core.agent_cancel_request(&request_id));
}

/// Takes a Note out of the agent's reach.
#[tauri::command(async)]
pub fn remove_agent_mark(app: AppHandle, id: String) -> Result<(), ApiError> {
    with_core(&app, |core| core.remove_agent_mark(&id))
}

// ---------- letting agents in (ADR-0012) ----------
//
// The owner's Start, Stop and status for the local route, and the agent account's name. None
// takes a path, uid, grant or duration: the screens can name an account, and only the native
// Start confirm, which names that account, lets anyone in.

/// For Settings: `state` is `off`, `running`, `ended_by_lock` or `expired`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct AgentRouteView {
    pub state: &'static str,
    pub expires_in_ms: Option<u64>,
    /// The Mac account named for the agent, if any.
    pub account: Option<String>,
    /// The code hash of the agent's tool the owner approved (ADR-0013), if any.
    pub tool_hash: Option<String>,
}

fn agent_route_view(app: &AppHandle) -> AgentRouteView {
    let core = core(app);
    let status = core.agent_grant_status();
    // A grant without its listener lets nobody in, so it isn't called running.
    let listening = app.state::<AgentRoute>().is_active();
    let running = status.state == "running" && listening;
    AgentRouteView {
        state: if status.state == "running" && !running {
            "off"
        } else {
            status.state
        },
        expires_in_ms: status.expires_in_ms.filter(|_| running),
        account: core.agent_account(),
        tool_hash: core.agent_tool_hash(),
    }
}

#[tauri::command(async)]
pub fn agent_route_status(app: AppHandle) -> AgentRouteView {
    agent_route_view(&app)
}

/// Names the agent's Mac account (`None` or `""` clears it). Ends a running grant.
#[tauri::command(async)]
pub fn set_agent_account(
    app: AppHandle,
    account: Option<String>,
) -> Result<AgentRouteView, ApiError> {
    with_core(&app, |core| core.set_agent_account(account))?;
    app.state::<AgentRoute>().stop();
    Ok(agent_route_view(&app))
}

/// Approves the agent tool's code hash (`None` or `""` clears it). Ends a running grant.
#[tauri::command(async)]
pub fn set_agent_tool_hash(
    app: AppHandle,
    hash: Option<String>,
) -> Result<AgentRouteView, ApiError> {
    with_core(&app, |core| core.set_agent_tool_hash(hash))?;
    app.state::<AgentRoute>().stop();
    Ok(agent_route_view(&app))
}

const AGENT_ROUTE_START: &str = "Start";

/// What the owner agrees to in the Start dialog. `who` names the account let in, and `which`
/// the programs in it that may ask.
fn agent_route_start_text(which: &str, who: &str) -> String {
    format!(
        "For up to one hour, {which} in the Mac user {who} can ask this app to store keys, \
         list and change the details of the keys they added, and ask to replace or delete one.\n\n\
         You type every value here, and the agent never sees one. Changing where a key is used, \
         replacing a value and deleting always ask you first.\n\n\
         The hour ends sooner if the Vault locks, and Stop ends it at once."
    )
}

/// The owner's explicit Start: one grant of at most an hour, only while Unlocked, for the
/// account named in Settings. The owner confirms in an OS dialog outside the WebView, so a
/// script in the page can ask but can't answer, and the dialog names who is let in.
#[tauri::command]
pub async fn agent_route_start(app: AppHandle) -> Result<AgentRouteView, ApiError> {
    require_unlocked(&app)?;
    if DIALOG_OPEN.load(Ordering::SeqCst) || crate::native_coordinator::busy(&app) {
        return Err(crate::core::not_now());
    }
    // Checked before the dialog, so the owner hears what's missing first; checked again by
    // the listener when it binds.
    let account = core(&app).agent_account();
    let tool_hash = core(&app).agent_tool_hash();
    let uid = agent_route::expected_uid(account.as_deref()).map_err(RouteError::api_error)?;
    let programs =
        agent_route::program_check(tool_hash.as_deref()).map_err(RouteError::api_error)?;
    let dir = agent_route::socket_dir();
    #[cfg(target_os = "macos")]
    agent_route::check_folder(&dir).map_err(RouteError::api_error)?;
    let dev_override = cfg!(debug_assertions) && std::env::var_os("ENOTE_DEV_AGENT_UID").is_some();
    let who = match &account {
        Some(name) if !dev_override => format!("“{name}”"),
        _ => format!("with uid {uid} (developer override)"),
    };
    let which = match &programs {
        agent_route::ProgramCheck::Required(_) => "Claude Code or Codex",
        #[cfg(debug_assertions)]
        agent_route::ProgramCheck::Off => "any program (developer override)",
    };
    let description = agent_route_start_text(which, &who);
    let window = app.get_webview_window("main");
    let answer = with_native_dialog(&app, move || {
        // A sheet on the app's own window; Cancel first because Return presses the first button.
        let window = window?;
        Some(
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Let the agent store keys?")
                .set_description(description)
                .set_buttons(rfd::MessageButtons::OkCancelCustom(
                    "Cancel".to_owned(),
                    AGENT_ROUTE_START.to_owned(),
                ))
                .set_parent(&window)
                .show(),
        )
    })
    .await
    .flatten();
    if !matches!(answer, Some(rfd::MessageDialogResult::Custom(button)) if button == AGENT_ROUTE_START)
    {
        return Err(ApiError {
            code: "cancelled",
            message: "Agents weren't let in.".to_owned(),
        });
    }
    // The Vault may have locked, or the account or tool changed, while the dialog was open.
    require_unlocked(&app)?;
    if core(&app).agent_account() != account || core(&app).agent_tool_hash() != tool_hash {
        return Err(crate::core::not_now());
    }
    let grant = with_core(&app, AppCore::agent_grant_start)?;
    let host = std::sync::Arc::new(agent_route::TauriHost(app.clone()));
    let started = app.state::<AgentRoute>().start(
        host,
        grant.id,
        uid,
        programs,
        &dir,
        agent_route::Limits::PRODUCTION,
    );
    if let Err(error) = started {
        with_core(&app, AppCore::agent_grant_stop);
        return Err(error.api_error());
    }
    Ok(agent_route_view(&app))
}

/// Ends the grant and the listener at once. A request waiting for the owner ends as cancelled.
#[tauri::command(async)]
pub fn agent_route_stop(app: AppHandle) -> AgentRouteView {
    with_core(&app, AppCore::agent_grant_stop);
    app.state::<AgentRoute>().stop();
    agent_route_view(&app)
}

// ---------- showing & copying ----------

#[tauri::command(async)]
pub fn show_field(
    app: AppHandle,
    id: String,
    field: String,
) -> Result<Zeroizing<String>, ApiError> {
    with_core(&app, |core| core.show_field(&id, &field))
}

#[tauri::command]
pub async fn show_wallet_field(
    app: AppHandle,
    id: String,
    field: String,
    mode: ShowMode,
) -> Result<(), ApiError> {
    native_unit(app, NativeAction::ShowWalletField { id, field, mode }).await
}

#[tauri::command]
pub async fn show_and_copy_wallet_field(
    app: AppHandle,
    id: String,
    field: String,
) -> Result<(), ApiError> {
    native_unit(app, NativeAction::ShowAndCopyWalletField { id, field }).await
}

#[tauri::command(async)]
pub fn hide_field(app: AppHandle) {
    crate::native_coordinator::cancel(&app);
    with_core(&app, AppCore::hide_field);
}

#[tauri::command(async)]
pub fn extend_reveal(app: AppHandle) -> Result<(), ApiError> {
    with_core(&app, AppCore::extend_reveal)
}

#[tauri::command]
pub async fn copy_field(app: AppHandle, id: String, field: String) -> Result<(), ApiError> {
    native_unit(app, NativeAction::CopyField { id, field }).await
}

#[tauri::command(async)]
pub fn copy_visible_field(app: AppHandle, id: String, field: String) -> Result<(), ApiError> {
    with_core(&app, |core| core.copy_visible_field(&id, &field))
}

#[tauri::command(async)]
pub fn copy_field_line(
    app: AppHandle,
    id: String,
    field: String,
    line: u32,
) -> Result<(), ApiError> {
    // A line number too large for this computer's `usize` is past the end of any value.
    let line = usize::try_from(line).unwrap_or(usize::MAX);
    with_core(&app, |core| core.copy_field_line(&id, &field, line))
}

#[tauri::command(async)]
pub fn copy_field_env(app: AppHandle, id: String) -> Result<(), ApiError> {
    with_core(&app, |core| core.copy_field_env(&id))
}

// ---------- checks ----------

#[tauri::command(async)]
pub fn seed_wordlist() -> &'static [&'static str] {
    notes::seed::wordlist()
}

#[tauri::command(async)]
pub fn check_seed(app: AppHandle, phrase: Zeroizing<String>) -> Result<SeedCheck, ApiError> {
    core(&app).check_seed(&phrase)
}

/// `chain` is free text (the Note's chain field): a name the app doesn't know means no check.
#[tauri::command(async)]
pub fn check_private_key(
    app: AppHandle,
    chain: String,
    key: Zeroizing<String>,
) -> Result<Option<&'static str>, ApiError> {
    core(&app).check_private_key(&chain, &key)
}

// ---------- Backups & Safety Copies ----------

#[tauri::command]
pub async fn pick_backup_destination(app: AppHandle) -> Result<PickedFile, ApiError> {
    core(&app).backup_destination_allowed()?;
    let home = home_dir(&app);
    let name = backup_file_name(core(&app).unix_secs());
    let path = pick_file(&app, move || {
        rfd::FileDialog::new()
            .set_title("Back up now")
            .set_directory(crate::platform::default_backup_folder(&home))
            .set_file_name(name)
            .add_filter("encrypted-note Backup", &["enote"])
            .save_file()
    })
    .await?;
    with_core(&app, |core| core.set_backup_destination(path))
}

#[tauri::command(async)]
pub fn forget_backup_destination(app: AppHandle) {
    with_core(&app, AppCore::forget_backup_destination);
}

#[tauri::command]
pub async fn write_backup(app: AppHandle) -> Result<BackupWritten, ApiError> {
    blocking(app, AppCore::write_backup).await
}

#[tauri::command]
pub async fn pick_backup_to_open(app: AppHandle) -> Result<PickedFile, ApiError> {
    core(&app).backup_to_open_allowed()?;
    let home = home_dir(&app);
    let path = pick_file(&app, move || {
        rfd::FileDialog::new()
            .set_title("Open a Backup")
            .set_directory(crate::platform::default_backup_folder(&home))
            .add_filter("encrypted-note Backup", &["enote"])
            .pick_file()
    })
    .await?;
    with_core(&app, |core| core.set_backup_to_open(path))
}

#[tauri::command]
pub async fn inspect_backup(
    app: AppHandle,
    kind: CredentialKind,
) -> Result<VaultPreview, ApiError> {
    match crate::native_coordinator::run(
        app,
        NativeAction::InspectBackup {
            recovery: kind.is_recovery(),
        },
    )
    .await?
    {
        NativeResult::Preview(preview) => Ok(preview),
        _ => Err(internal()),
    }
}

#[tauri::command(async)]
pub fn replace_with_backup(app: AppHandle) -> Result<(), ApiError> {
    with_core(&app, AppCore::replace_with_backup)
}

#[tauri::command(async)]
pub fn discard_inspection(app: AppHandle) {
    with_core(&app, AppCore::discard_inspection);
}

#[tauri::command(async)]
pub fn list_safety_copies(app: AppHandle) -> Result<Vec<SafetyCopy>, ApiError> {
    core(&app).list_safety_copies()
}

#[tauri::command]
pub async fn inspect_safety_copy(
    app: AppHandle,
    id: String,
    kind: CredentialKind,
) -> Result<VaultPreview, ApiError> {
    match crate::native_coordinator::run(
        app,
        NativeAction::InspectSafetyCopy {
            id,
            recovery: kind.is_recovery(),
        },
    )
    .await?
    {
        NativeResult::Preview(preview) => Ok(preview),
        _ => Err(internal()),
    }
}

#[tauri::command(async)]
pub fn restore_safety_copy(app: AppHandle) -> Result<(), ApiError> {
    with_core(&app, AppCore::restore_safety_copy)
}

// ---------- account care & settings ----------

#[tauri::command]
pub async fn change_password(app: AppHandle, rotate: bool) -> Result<(), ApiError> {
    native_unit(app, NativeAction::ChangePassword { rotate }).await
}

#[tauri::command]
pub async fn set_settings(
    app: AppHandle,
    settings: SettingsRequest,
) -> Result<SettingsView, ApiError> {
    match crate::native_coordinator::run(app, NativeAction::SetSettings(settings.into())).await? {
        NativeResult::Settings(settings) => Ok(settings),
        _ => Err(internal()),
    }
}

#[tauri::command(async)]
pub fn mark_seed_explainer_seen(app: AppHandle) -> Result<(), ApiError> {
    with_core(&app, AppCore::mark_seed_explainer_seen)
}

/// Where this computer's Vault is kept, as text to show. Read-only, in any phase.
#[tauri::command(async)]
pub fn vault_location(app: AppHandle) -> String {
    core(&app).vault_location()
}

/// Show this computer's Vault in Finder or File Explorer. No argument of any kind: the screens
/// can't choose what opens. `DIALOG_OPEN` stays off because this is a real switch to another app,
/// so losing focus still cancels native work and locks if the owner chose that.
#[tauri::command(async)]
pub fn reveal_vault_folder(app: AppHandle) -> Result<(), ApiError> {
    if crate::native_coordinator::busy(&app) {
        return Err(crate::core::not_now());
    }
    let target = with_core(&app, AppCore::vault_reveal_target)?;
    let (tx, rx) = mpsc::channel();
    let opened = app
        .run_on_main_thread(move || {
            let _ = tx.send(crate::platform::reveal_in_file_manager(&target));
        })
        .is_ok()
        && rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false);
    if opened {
        Ok(())
    } else {
        Err(ApiError {
            code: "io",
            message: "The folder couldn't be opened. You can copy the path and open it yourself."
                .to_owned(),
        })
    }
}

#[tauri::command(async)]
pub fn dismiss_sharing_guard(app: AppHandle) {
    with_core(&app, AppCore::dismiss_sharing_guard);
}

// ---------- dialogs ----------

/// Show a native dialog off the main thread (rfd hands it to the main thread itself) and wait for
/// the owner.
///
/// Focus changes are ignored while it is open, so a shown value is hidden first; switching to
/// another app meanwhile still arrives as an app-switch notification (`platform`).
async fn with_native_dialog<T: Send + 'static>(
    app: &AppHandle,
    dialog: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    with_core(app, AppCore::hide_field);
    DIALOG_OPEN.store(true, Ordering::SeqCst);
    let answer = tauri::async_runtime::spawn_blocking(dialog).await;
    DIALOG_OPEN.store(false, Ordering::SeqCst);
    answer.ok()
}

/// A native file dialog. Cancelling is the `cancelled` error the screens ignore.
async fn pick_file(
    app: &AppHandle,
    dialog: impl FnOnce() -> Option<PathBuf> + Send + 'static,
) -> Result<PathBuf, ApiError> {
    with_native_dialog(app, dialog)
        .await
        .flatten()
        .ok_or_else(|| ApiError {
            code: "cancelled",
            message: "Nothing was chosen.".to_owned(),
        })
}

/// The owner's real home folder (not the App Sandbox container), as the core was given it.
fn home_dir(app: &AppHandle) -> PathBuf {
    core(app).home_dir().to_path_buf()
}

/// `encrypted-note-backup-YYYY-MM-DD-HHMM.enote` (UTC date and time), so a second Backup on one
/// day doesn't land on the first one's name, which the app can't safely replace.
fn backup_file_name(unix_secs: i64) -> String {
    let (y, m, d) = civil_date(unix_secs);
    let mins = unix_secs.rem_euclid(86_400) / 60;
    format!(
        "encrypted-note-backup-{y:04}-{m:02}-{d:02}-{:02}{:02}.enote",
        mins / 60,
        mins % 60
    )
}

/// Unix seconds to (year, month, day) in UTC (Howard Hinnant's `civil_from_days`).
fn civil_date(unix_secs: i64) -> (i64, i64, i64) {
    let z = unix_secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

// ---------- Agent Companion (ADR-0009) ----------
//
// Four commands, none taking an argument: the screens cannot choose an executable, script,
// destination, model, credential, path or grant. Replies are fixed nonsecret fields only.

fn require_unlocked(app: &AppHandle) -> Result<(), ApiError> {
    if core(app).app_status().phase == "unlocked" {
        Ok(())
    } else {
        Err(ApiError {
            code: "locked",
            message: "Unlock the Vault first.".to_owned(),
        })
    }
}

#[tauri::command(async)]
pub fn companion_status(app: AppHandle) -> CompanionView {
    app.state::<Companion>().status()
}

const COMPANION_START: &str = "Start";

/// What the owner agrees to in the Start dialog.
fn companion_start_text(idle_minutes: u32, count_activity_anywhere: bool) -> String {
    let minutes = if idle_minutes == 1 {
        "1 minute".to_owned()
    } else {
        format!("{idle_minutes} minutes")
    };
    let place = if count_activity_anywhere {
        "computer"
    } else {
        "window"
    };
    format!(
        "For up to one hour, any program in the agent's Mac user can send text to OpenAI \
         through the helper. That can cost money, and what it sends leaves this computer.\n\n\
         The hour ends sooner if the Vault locks: Auto-lock is set to {minutes} without using this \
         {place}. So keeping the tool running means keeping your Vault Unlocked.\n\n\
         Stop ends new requests at once. It can't undo a request already sent or billed, and it \
         doesn't revoke the Agent Key at OpenAI."
    )
}

/// Only the Start button counts: closing the dialog, or anything unexpected, doesn't start it.
fn companion_start_confirmed(answer: Option<rfd::MessageDialogResult>) -> bool {
    matches!(answer, Some(rfd::MessageDialogResult::Custom(button)) if button == COMPANION_START)
}

/// The owner's explicit Start: one new grant of at most an hour, only while Unlocked. The owner
/// confirms in an OS dialog outside the WebView, so a script in the page can ask but can't answer.
#[tauri::command]
pub async fn companion_start(app: AppHandle) -> Result<CompanionView, ApiError> {
    require_unlocked(&app)?;
    if !cfg!(target_os = "macos") {
        return Ok(app.state::<Companion>().status());
    }
    if DIALOG_OPEN.load(Ordering::SeqCst) || crate::native_coordinator::busy(&app) {
        return Err(crate::core::not_now());
    }
    let settings = core(&app).app_status().settings;
    let description = companion_start_text(settings.idle_minutes, settings.count_activity_anywhere);
    let window = app.get_webview_window("main");
    let answer = with_native_dialog(&app, move || {
        // A sheet on the app's own window, so the alert taking focus doesn't count as leaving the
        // app. Cancel comes first because Return presses the first button.
        let window = window?;
        Some(
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Warning)
                .set_title("Start the Agent Companion?")
                .set_description(description)
                .set_buttons(rfd::MessageButtons::OkCancelCustom(
                    "Cancel".to_owned(),
                    COMPANION_START.to_owned(),
                ))
                .set_parent(&window)
                .show(),
        )
    })
    .await
    .flatten();
    if !companion_start_confirmed(answer) {
        return Err(ApiError {
            code: "cancelled",
            message: "The Agent Companion wasn't started.".to_owned(),
        });
    }
    // The Vault may have locked while the dialog was open.
    require_unlocked(&app)?;
    tauri::async_runtime::spawn_blocking(move || app.state::<Companion>().start())
        .await
        .map_err(|_| ApiError {
            code: "internal",
            message: "The Agent Companion didn't start. Press Start again.".to_owned(),
        })
}

#[tauri::command(async)]
pub fn companion_stop(app: AppHandle) -> CompanionView {
    app.state::<Companion>().stop()
}

/// Native owner setup: the owner picks the reviewed companion plan and the separately created
/// Agent Key file in OS-drawn panels. This process opens the two files and hands the helper
/// their descriptors; it never reads them, and nothing about them returns to the screens.
#[tauri::command]
pub async fn companion_setup(app: AppHandle) -> Result<CompanionView, ApiError> {
    require_unlocked(&app)?;
    if !cfg!(target_os = "macos") {
        return Ok(app.state::<Companion>().status());
    }
    let home = home_dir(&app);
    let plan = pick_file(&app, {
        let home = home.clone();
        move || {
            rfd::FileDialog::new()
                .set_title("Choose the Agent Companion plan file.")
                .set_directory(&home)
                .add_filter("Companion plan", &["json"])
                .pick_file()
        }
    })
    .await?;
    let credential = pick_file(&app, move || {
        rfd::FileDialog::new()
            .set_title("Choose the separate Agent Key file (never a key from your Vault)")
            .set_directory(&home)
            .pick_file()
    })
    .await?;
    // The Vault may have locked while the panels were open.
    require_unlocked(&app)?;
    let plan = open_chosen(&plan)?;
    let credential = open_chosen(&credential)?;
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Companion>().install(&plan, &credential)
    })
    .await
    .map_err(|_| ApiError {
        code: "internal",
        message: "The Agent Companion setup didn't finish. Try Set up… again.".to_owned(),
    })
}

/// Opens an owner-chosen file read-only without following a symbolic link.
fn open_chosen(path: &std::path::Path) -> Result<std::fs::File, ApiError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path).map_err(|_| ApiError {
        code: "io",
        message: "That file couldn't be opened. Check that it still exists and isn't a link to another file, then choose it again.".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        COMPANION_START, CoreState, SettingsRequest, backup_file_name, companion_start_confirmed,
        companion_start_text,
    };
    use crate::core::native::NativeAction;
    use crate::core::{ApiError, AppCore, Clipboard, Config};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use zeroize::Zeroizing;

    struct CountingClipboard(Arc<AtomicU64>);

    impl Clipboard for CountingClipboard {
        fn write(&mut self, _text: &str) -> Result<u64, ApiError> {
            Ok(self.0.fetch_add(1, Ordering::SeqCst) + 1)
        }

        fn clear_if(&mut self, _change_count: u64) -> bool {
            true
        }
    }

    #[test]
    fn show_and_copy_use_focus_before_the_event_worker_runs() {
        let temp = tempfile::tempdir().unwrap();
        let copies = Arc::new(AtomicU64::new(0));
        let mut core = AppCore::new(Config {
            data_dir: temp.path().join("fictional-vault"),
            home_dir: temp.path().join("fictional-home"),
            limits: vault_format::Limits::TESTING,
            clock: Box::new(crate::SystemClock::start()),
            clipboard: Box::new(CountingClipboard(copies.clone())),
            activity_anywhere_supported: false,
        })
        .unwrap();
        let recovery = core
            .create_vault(
                Zeroizing::new("fake-lantern-quartz-mosaic-velvet-orbit".to_owned()),
                vault_format::KdfParams::TESTING,
            )
            .unwrap()
            .recovery_key
            .unwrap();
        core.confirm_recovery_kit(&recovery[recovery.len() - 9..])
            .unwrap();
        let id = core
            .create_note(
                serde_json::from_value(serde_json::json!({
                    "kind": "api_key", "title": "Fictional focus test",
                    "visible": {"service": "Fictional provider", "env_var": "FAKE_AGENT_KEY"},
                    "hidden": {"key": "FAKE_KEY_FOR_FOCUS_TESTS_ONLY"}
                }))
                .unwrap(),
                None,
            )
            .unwrap();
        let state = CoreState(Mutex::new(core));
        let focused = AtomicBool::new(true);
        {
            let mut core = state.lock(&focused);
            core.show_field(&id, "key").unwrap();
            core.hide_field();
        }

        // The OS recorded the loss, but its worker has not yet acquired CoreState.
        focused.store(false, Ordering::SeqCst);
        {
            let mut core = state.lock(&focused);
            let error = core.show_field(&id, "key").unwrap_err();
            assert_eq!(error.code, "invalid_input");
            assert!(error.message.contains("window is in front"));
            for result in [
                core.copy_field(&id, "key", None),
                core.copy_field_line(&id, "key", 0),
                core.copy_field_env(&id),
                core.copy_visible_field(&id, "service"),
            ] {
                assert_eq!(result.unwrap_err().code, "invalid_input");
            }
            assert_eq!(
                core.begin_native(NativeAction::CopyField {
                    id: id.clone(),
                    field: "key".to_owned(),
                })
                .err()
                .unwrap()
                .code,
                "invalid_input"
            );
            assert_eq!(core.native_pending_id(), None);
            assert_eq!(core.app_status().reveal, None);
        }
        assert_eq!(copies.load(Ordering::SeqCst), 0);

        // The gain can also be newer than the last focus event applied to the core.
        state
            .0
            .lock()
            .unwrap()
            .os_event(session::OsEvent::FocusLost);
        focused.store(true, Ordering::SeqCst);
        let mut core = state.lock(&focused);
        core.show_field(&id, "key").unwrap();
        core.copy_field(&id, "key", None).unwrap();
        assert_eq!(copies.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn only_the_start_button_starts_the_companion() {
        use rfd::MessageDialogResult::{Cancel, Custom, No, Ok, Yes};
        assert!(companion_start_confirmed(Some(Custom(
            COMPANION_START.to_owned()
        ))));
        for answer in [
            None,
            Some(Custom("Cancel".to_owned())),
            Some(Custom(String::new())),
            Some(Cancel),
            Some(Ok),
            Some(Yes),
            Some(No),
        ] {
            assert!(!companion_start_confirmed(answer));
        }
    }

    #[test]
    fn the_start_dialog_says_what_the_owner_is_agreeing_to() {
        let window = companion_start_text(5, false);
        assert!(window.contains("For up to one hour"));
        assert!(window.contains("can cost money"));
        assert!(window.contains("Auto-lock is set to 5 minutes without using this window."));
        assert!(window.contains("doesn't revoke the Agent Key"));
        assert!(window.contains("any program in the agent's Mac user"));
        assert!(window.contains("keeping your Vault Unlocked"));
        let computer = companion_start_text(1, true);
        assert!(computer.contains("Auto-lock is set to 1 minute without using this computer."));
    }

    #[test]
    fn webview_settings_cannot_supply_a_master_password() {
        let ordinary = serde_json::json!({"idle_minutes":10,"lock_on_app_switch":false});
        assert!(serde_json::from_value::<SettingsRequest>(ordinary.clone()).is_ok());
        for name in ["master_password", "recovery_key"] {
            let mut credential_bearing = ordinary.clone();
            credential_bearing[name] = serde_json::json!("obviously-fake-credential");
            assert!(serde_json::from_value::<SettingsRequest>(credential_bearing).is_err());
        }
    }

    #[test]
    fn backup_file_names_carry_the_utc_date_and_time() {
        assert_eq!(
            backup_file_name(0),
            "encrypted-note-backup-1970-01-01-0000.enote"
        );
        // 2024-02-29 23:59:59 UTC (leap day).
        assert_eq!(
            backup_file_name(1_709_251_199),
            "encrypted-note-backup-2024-02-29-2359.enote"
        );
        // 2026-09-23 12:00:00 UTC.
        assert_eq!(
            backup_file_name(1_790_164_800),
            "encrypted-note-backup-2026-09-23-1200.enote"
        );
        // Before 1970, the time of day still counts forward from midnight.
        assert_eq!(
            backup_file_name(-60),
            "encrypted-note-backup-1969-12-31-2359.enote"
        );
    }
}
