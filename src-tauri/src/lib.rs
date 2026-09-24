//! encrypted-note desktop app: the Tauri shell around the core crates.
//!
//! Trust boundary (ADR-0003): this Rust process owns every key, the decrypted document, the
//! Vault file and all OS integrations. The webview is untrusted. It gets visible Note fields and,
//! only on Show, one Hidden Field value at a time. Copy happens entirely here.
//!
//! * [`core`]: all behaviour, as plain Rust (tested without Tauri).
//! * `commands`: the `#[tauri::command]` wrappers the webview may call.
//! * `platform`: macOS and Windows adapters (Capture Hiding self-check, running apps, clipboard,
//!   lock events).
//! * This file: startup, window events, and the background thread that ticks the session every
//!   500 ms and polls running apps and the Capture Hiding self-check every 2 s.

#![deny(unsafe_code)]

mod commands;
pub mod core;
#[allow(unsafe_code)]
mod hardening;
#[allow(unsafe_code)]
mod platform;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use session::OsEvent;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Window, WindowEvent};

use crate::commands::{CoreState, DIALOG_OPEN, STATUS_EVENT};
use crate::core::{AppCore, AppStatus, Clock, Config};

pub fn run() {
    hardening::apply();

    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::activity,
            commands::suggest_passphrase,
            commands::assess_password,
            commands::create_vault,
            commands::confirm_recovery_kit,
            commands::unlock,
            commands::set_new_password,
            commands::lock,
            commands::list_notes,
            commands::get_note,
            commands::create_note,
            commands::update_note,
            commands::trash_note,
            commands::restore_note,
            commands::delete_forever,
            commands::empty_trash,
            commands::set_favorite,
            commands::show_field,
            commands::show_wallet_field,
            commands::hide_field,
            commands::copy_field,
            commands::seed_wordlist,
            commands::check_seed,
            commands::check_private_key,
            commands::pick_backup_destination,
            commands::write_backup,
            commands::pick_backup_to_open,
            commands::inspect_backup,
            commands::replace_with_backup,
            commands::list_safety_copies,
            commands::inspect_safety_copy,
            commands::restore_safety_copy,
            commands::change_password,
            commands::set_settings,
            commands::mark_seed_explainer_seen,
            commands::dismiss_sharing_guard,
        ])
        .setup(|app| {
            setup(app.handle());
            Ok(())
        })
        .on_window_event(on_window_event)
        .build(tauri::generate_context!())
        .expect("failed to start encrypted-note");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            // Clears the clipboard if it still holds our copy, and drops the keys.
            commands::os_event(app, OsEvent::Quit);
        }
    });
}

fn setup(app: &AppHandle) {
    let core = data_dir(app)
        .and_then(|data_dir| {
            let home_dir = app.path().home_dir().map_err(|e| e.to_string())?;
            AppCore::new(Config {
                data_dir,
                home_dir,
                limits: vault_format::Limits::PRODUCTION,
                clock: Box::new(SystemClock(Instant::now())),
                clipboard: Box::new(platform::SystemClipboard),
            })
            .map_err(|e| e.message)
        })
        .unwrap_or_else(|message| fatal(&message));
    app.manage(CoreState(Mutex::new(core)));

    let handle = app.clone();
    platform::observe_lock_events(app, move |event| {
        // Never block the main thread on the core (an unlock may be running Argon2id).
        let handle = handle.clone();
        std::thread::spawn(move || commands::os_event(&handle, event));
    });

    if dev_allow_capture()
        && let Some(window) = app.get_webview_window("main")
    {
        let _ = window.set_content_protected(false);
    }

    let handle = app.clone();
    std::thread::spawn(move || background(&handle));
}

/// Developers only: `ENOTE_DEV_ALLOW_CAPTURE=1` turns Capture Hiding off in debug builds, since
/// screenshots of the app are blank otherwise (ADR-0002). Always false in release builds.
fn dev_allow_capture() -> bool {
    cfg!(debug_assertions) && std::env::var("ENOTE_DEV_ALLOW_CAPTURE").as_deref() == Ok("1")
}

/// Mac: `<app data>/Vault.noindex` (the suffix keeps Spotlight out). Windows: LocalAppData, not
/// Roaming, so the Vault never follows a roaming profile.
fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    // Developers only: test runs must never touch the owner's real Vault folder.
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("ENOTE_DEV_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    #[cfg(windows)]
    let base = app.path().app_local_data_dir();
    #[cfg(not(windows))]
    let base = app.path().app_data_dir();
    base.map(|dir| dir.join("Vault.noindex"))
        .map_err(|e| e.to_string())
}

/// A plain message, then quit (e.g. a second copy of the app is already running).
fn fatal(message: &str) -> ! {
    rfd::MessageDialog::new()
        .set_title("encrypted-note")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .show();
    std::process::exit(1);
}

fn on_window_event(window: &Window, event: &WindowEvent) {
    let event = match event {
        // Our own Backup file dialog taking focus isn't switching apps.
        WindowEvent::Focused(false) if DIALOG_OPEN.load(Ordering::SeqCst) => return,
        WindowEvent::Focused(false) => OsEvent::FocusLost,
        WindowEvent::Focused(true) => OsEvent::FocusGained,
        _ => return,
    };
    let app = window.app_handle().clone();
    // Off the main thread: the core may be busy with Argon2id for a second.
    std::thread::spawn(move || commands::os_event(&app, event));
}

/// Tick every 500 ms; every 2 s also poll running apps and the Capture Hiding self-check. Emit
/// "status-changed" whenever the status changes in more than its countdowns (the screens poll
/// once a second for those).
fn background(app: &AppHandle) {
    let mut last: Option<AppStatus> = None;
    for round in 0u64.. {
        std::thread::sleep(Duration::from_millis(500));
        let probe = if round % 4 == 0 {
            probe_platform(app)
        } else {
            None
        };
        let status = {
            let mut core = commands::core(app);
            if let Some((apps, capture_hiding_active)) = probe {
                core.guard_update(&apps);
                core.set_capture_hiding_active(capture_hiding_active);
            }
            core.tick();
            core.app_status()
        };
        let key = without_countdowns(status.clone());
        if last.as_ref() != Some(&key) {
            let _ = app.emit(STATUS_EVENT, status);
            last = Some(key);
        }
    }
}

/// Running apps and the self-check, read on the main thread (AppKit requires it for NSWindow).
fn probe_platform(app: &AppHandle) -> Option<(Vec<guard::RunningApp>, Option<bool>)> {
    let (tx, rx) = mpsc::channel();
    let window = app.get_webview_window("main");
    app.run_on_main_thread(move || {
        let capture = window.as_ref().and_then(platform::capture_hiding_active);
        let _ = tx.send((platform::running_apps(), capture));
    })
    .ok()?;
    rx.recv_timeout(Duration::from_secs(2)).ok()
}

fn without_countdowns(mut status: AppStatus) -> AppStatus {
    for ms in [
        &mut status.lock_in_ms,
        &mut status.clipboard_clear_in_ms,
        &mut status.retry_in_ms,
    ] {
        *ms = ms.map(|_| 0);
    }
    if let Some(reveal) = &mut status.reveal {
        reveal.expires_in_ms = reveal.expires_in_ms.map(|_| 0);
    }
    status
}

struct SystemClock(Instant);

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn unix_secs(&self) -> i64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
    }
}
