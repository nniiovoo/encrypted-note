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

use crate::companion::{Companion, CompanionView};
use crate::core::{
    ApiError, AppCore, AppStatus, BackupWritten, NoteEditInput, NoteInput, PickedFile, SafetyCopy,
    SettingsInput, SettingsView, ShowMode, VaultPreview,
};

pub const STATUS_EVENT: &str = "status-changed";

/// Managed state: the one `AppCore`.
pub struct CoreState(pub Mutex<AppCore>);

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
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
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
        COMPANION_START, SettingsRequest, backup_file_name, companion_start_confirmed,
        companion_start_text,
    };

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
