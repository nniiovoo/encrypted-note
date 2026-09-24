//! The app bridge: one thin `#[tauri::command]` per function in `src/api.ts`, each a call into
//! [`AppCore`] followed by a "status-changed" event (read-only commands skip the event).
//!
//! No command runs on the main thread: Tauri runs plain `fn` commands there unless they are
//! marked `(async)`, and waiting for the core while an unlock runs Argon2id would freeze the
//! window. Commands that run Argon2id (about a second) or wait for a file dialog are `async fn`
//! and do that work on the blocking pool.
//!
//! The webview may call exactly these commands: `build.rs` lists them in the app manifest and
//! `capabilities/main.json` grants each one (plus listening to events).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use notes::keys::Chain;
use notes::seed::SeedCheck;
use notes::{Filter, NoteSummary, NoteView};
use session::OsEvent;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use zeroize::Zeroizing;

use crate::core::{
    ApiError, AppCore, AppStatus, AssessmentView, BackupWritten, CredentialInput, NoteEditInput,
    NoteInput, PickedFile, RecoveryKeyShown, SafetyCopy, SettingsInput, ShowMode, VaultPreview,
};

pub const STATUS_EVENT: &str = "status-changed";

/// Managed state: the one `AppCore`.
pub struct CoreState(pub Mutex<AppCore>);

/// True while a Backup file dialog is open. The dialog takes focus from our window, which must
/// not count as switching to another app (that could lock the Vault mid-backup).
pub static DIALOG_OPEN: AtomicBool = AtomicBool::new(false);

pub fn core<R: Runtime>(app: &AppHandle<R>) -> MutexGuard<'_, AppCore> {
    app.state::<CoreState>()
        .inner()
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Run `f` on the core, then tell the screens the new status.
pub fn with_core<R: Runtime, T>(app: &AppHandle<R>, f: impl FnOnce(&mut AppCore) -> T) -> T {
    let mut core = core(app);
    let out = f(&mut core);
    let _ = app.emit(STATUS_EVENT, core.app_status());
    out
}

/// [`with_core`] on the blocking pool.
async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&mut AppCore) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tauri::async_runtime::spawn_blocking(move || with_core(&app, f))
        .await
        .map_err(|_| internal())?
}

fn internal() -> ApiError {
    ApiError {
        code: "internal",
        message: "Something went wrong inside encrypted-note. Nothing was changed.".to_owned(),
    }
}

pub fn os_event<R: Runtime>(app: &AppHandle<R>, event: OsEvent) {
    with_core(app, |core| core.os_event(event));
}

// ---------- status & events ----------

#[tauri::command(async)]
pub fn app_status(app: AppHandle) -> AppStatus {
    core(&app).app_status()
}

#[tauri::command(async)]
pub fn activity(app: AppHandle) {
    core(&app).activity();
}

// ---------- setup, unlock, lock ----------

#[tauri::command(async)]
pub fn suggest_passphrase(app: AppHandle) -> Zeroizing<String> {
    core(&app).suggest_passphrase()
}

#[tauri::command(async)]
pub fn assess_password(app: AppHandle, password: Zeroizing<String>) -> AssessmentView {
    core(&app).assess_password(&password)
}

#[tauri::command]
pub async fn create_vault(
    app: AppHandle,
    master_password: Zeroizing<String>,
) -> Result<RecoveryKeyShown, ApiError> {
    // Calibrate first, without holding the core: it runs several Argon2id derivations, and the
    // screens keep polling the status meanwhile.
    let limits = vault_format::Limits::PRODUCTION;
    let params = tauri::async_runtime::spawn_blocking(move || {
        vault_format::calibrate(&limits, vault_format::measure_kdf)
    })
    .await
    .map_err(|_| internal())?;
    blocking(app, move |core| core.create_vault(master_password, params)).await
}

#[tauri::command(async)]
pub fn confirm_recovery_kit(
    app: AppHandle,
    last_groups: Zeroizing<String>,
) -> Result<(), ApiError> {
    with_core(&app, |core| core.confirm_recovery_kit(&last_groups))
}

#[tauri::command]
pub async fn unlock(app: AppHandle, credential: CredentialInput) -> Result<(), ApiError> {
    blocking(app, move |core| core.unlock(credential)).await
}

#[tauri::command]
pub async fn set_new_password(
    app: AppHandle,
    master_password: Zeroizing<String>,
) -> Result<(), ApiError> {
    blocking(app, move |core| core.set_new_password(master_password)).await
}

#[tauri::command(async)]
pub fn lock(app: AppHandle) {
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
pub async fn create_note(
    app: AppHandle,
    input: NoteInput,
    master_password: Option<Zeroizing<String>>,
) -> Result<String, ApiError> {
    blocking(app, move |core| core.create_note(input, master_password)).await
}

#[tauri::command]
pub async fn update_note(
    app: AppHandle,
    id: String,
    input: NoteEditInput,
    master_password: Option<Zeroizing<String>>,
) -> Result<(), ApiError> {
    blocking(app, move |core| {
        core.update_note(&id, input, master_password)
    })
    .await
}

#[tauri::command(async)]
pub fn trash_note(app: AppHandle, id: String) -> Result<(), ApiError> {
    with_core(&app, |core| core.trash_note(&id))
}

#[tauri::command(async)]
pub fn restore_note(app: AppHandle, id: String) -> Result<(), ApiError> {
    with_core(&app, |core| core.restore_note(&id))
}

#[tauri::command(async)]
pub fn delete_forever(app: AppHandle, id: String) -> Result<(), ApiError> {
    with_core(&app, |core| core.delete_forever(&id))
}

#[tauri::command(async)]
pub fn empty_trash(app: AppHandle) -> Result<usize, ApiError> {
    with_core(&app, AppCore::empty_trash)
}

#[tauri::command(async)]
pub fn set_favorite(app: AppHandle, id: String, favorite: bool) -> Result<(), ApiError> {
    with_core(&app, |core| core.set_favorite(&id, favorite))
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
    master_password: Zeroizing<String>,
    mode: ShowMode,
) -> Result<Zeroizing<String>, ApiError> {
    blocking(app, move |core| {
        core.show_wallet_field(&id, &field, master_password, mode)
    })
    .await
}

#[tauri::command(async)]
pub fn hide_field(app: AppHandle) {
    with_core(&app, AppCore::hide_field);
}

#[tauri::command]
pub async fn copy_field(
    app: AppHandle,
    id: String,
    field: String,
    master_password: Option<Zeroizing<String>>,
) -> Result<(), ApiError> {
    blocking(app, move |core| {
        core.copy_field(&id, &field, master_password)
    })
    .await
}

// ---------- checks ----------

#[tauri::command(async)]
pub fn seed_wordlist(app: AppHandle) -> &'static [&'static str] {
    core(&app).seed_wordlist()
}

#[tauri::command(async)]
pub fn check_seed(app: AppHandle, phrase: Zeroizing<String>) -> Result<SeedCheck, ApiError> {
    core(&app).check_seed(&phrase)
}

#[tauri::command(async)]
pub fn check_private_key(
    app: AppHandle,
    chain: Chain,
    key: Zeroizing<String>,
) -> Result<Option<&'static str>, ApiError> {
    core(&app).check_private_key(chain, &key)
}

// ---------- Backups & Safety Copies ----------

#[tauri::command]
pub async fn pick_backup_destination(app: AppHandle) -> Result<PickedFile, ApiError> {
    core(&app).backup_destination_allowed()?;
    let home = home_dir(&app)?;
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

#[tauri::command]
pub async fn write_backup(app: AppHandle) -> Result<BackupWritten, ApiError> {
    blocking(app, AppCore::write_backup).await
}

#[tauri::command]
pub async fn pick_backup_to_open(app: AppHandle) -> Result<PickedFile, ApiError> {
    core(&app).backup_to_open_allowed()?;
    let home = home_dir(&app)?;
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
    credential: CredentialInput,
) -> Result<VaultPreview, ApiError> {
    blocking(app, move |core| core.inspect_backup(credential)).await
}

#[tauri::command(async)]
pub fn replace_with_backup(app: AppHandle) -> Result<(), ApiError> {
    with_core(&app, AppCore::replace_with_backup)
}

#[tauri::command(async)]
pub fn list_safety_copies(app: AppHandle) -> Result<Vec<SafetyCopy>, ApiError> {
    core(&app).list_safety_copies()
}

#[tauri::command]
pub async fn inspect_safety_copy(
    app: AppHandle,
    id: String,
    credential: CredentialInput,
) -> Result<VaultPreview, ApiError> {
    blocking(app, move |core| core.inspect_safety_copy(&id, credential)).await
}

#[tauri::command(async)]
pub fn restore_safety_copy(app: AppHandle) -> Result<(), ApiError> {
    with_core(&app, AppCore::restore_safety_copy)
}

// ---------- account care & settings ----------

#[tauri::command]
pub async fn change_password(
    app: AppHandle,
    current_password: Zeroizing<String>,
    new_password: Zeroizing<String>,
    rotate: bool,
) -> Result<RecoveryKeyShown, ApiError> {
    blocking(app, move |core| {
        core.change_password(current_password, new_password, rotate)
    })
    .await
}

#[tauri::command(async)]
pub fn set_settings(app: AppHandle, settings: SettingsInput) -> Result<(), ApiError> {
    with_core(&app, |core| core.set_settings(settings))
}

#[tauri::command(async)]
pub fn mark_seed_explainer_seen(app: AppHandle) -> Result<(), ApiError> {
    with_core(&app, AppCore::mark_seed_explainer_seen)
}

#[tauri::command(async)]
pub fn dismiss_sharing_guard(app: AppHandle) {
    with_core(&app, AppCore::dismiss_sharing_guard);
}

// ---------- dialogs ----------

/// Show a native file dialog off the main thread (rfd hands it to the main thread itself) and
/// wait for the owner. Cancelling is the `cancelled` error the screens ignore.
///
/// Focus changes are ignored while it is open, so a shown value is hidden first; switching to
/// another app meanwhile still arrives as an app-switch notification (`platform`).
async fn pick_file(
    app: &AppHandle,
    dialog: impl FnOnce() -> Option<PathBuf> + Send + 'static,
) -> Result<PathBuf, ApiError> {
    with_core(app, AppCore::hide_field);
    DIALOG_OPEN.store(true, Ordering::SeqCst);
    let picked = tauri::async_runtime::spawn_blocking(dialog).await;
    DIALOG_OPEN.store(false, Ordering::SeqCst);
    picked.ok().flatten().ok_or_else(|| ApiError {
        code: "cancelled",
        message: "Nothing was chosen.".to_owned(),
    })
}

fn home_dir(app: &AppHandle) -> Result<PathBuf, ApiError> {
    app.path().home_dir().map_err(|_| ApiError {
        code: "io",
        message: "encrypted-note couldn't find your home folder.".to_owned(),
    })
}

/// `encrypted-note-backup-YYYY-MM-DD.enote` (UTC date).
fn backup_file_name(unix_secs: i64) -> String {
    let (y, m, d) = civil_date(unix_secs);
    format!("encrypted-note-backup-{y:04}-{m:02}-{d:02}.enote")
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

#[cfg(test)]
mod tests {
    use super::backup_file_name;

    #[test]
    fn backup_file_names_carry_the_utc_date() {
        assert_eq!(
            backup_file_name(0),
            "encrypted-note-backup-1970-01-01.enote"
        );
        // 2024-02-29 23:59:59 UTC (leap day).
        assert_eq!(
            backup_file_name(1_709_251_199),
            "encrypted-note-backup-2024-02-29.enote"
        );
        // 2026-09-23 12:00:00 UTC.
        assert_eq!(
            backup_file_name(1_790_164_800),
            "encrypted-note-backup-2026-09-23.enote"
        );
    }
}
