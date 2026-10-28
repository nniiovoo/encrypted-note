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
//! * `wipe_on_free`: the global allocator, which wipes every block as it is freed.
//! * This file: startup, the rule that the window may load nothing but the app itself, window
//!   events, and the background thread that ticks the session every 500 ms and polls running
//!   apps and the Capture Hiding self-check every 2 s.

#![deny(unsafe_code)]

#[allow(unsafe_code)]
pub mod biometry;
mod commands;
#[allow(unsafe_code)]
pub mod companion;
pub mod core;
#[allow(unsafe_code)]
mod hardening;
mod native_coordinator;
#[allow(unsafe_code)]
pub mod native_credential;
#[allow(unsafe_code)]
mod platform;
mod quick_runtime;
pub mod quick_unlock;
#[allow(unsafe_code)]
mod wipe_on_free;

/// Passwords and Hidden Field values pass through buffers we don't own (Tauri's IPC layer,
/// `serde_json::Value`, zxcvbn) on their way to and from our wiping types. Those buffers are
/// freed through this allocator, which zeroes them first.
#[global_allocator]
static ALLOCATOR: wipe_on_free::WipeOnFree = wipe_on_free::WipeOnFree;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use session::OsEvent;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Url, Window, WindowEvent};

use crate::commands::{CoreState, DIALOG_OPEN, STATUS_EVENT};
use crate::core::{AppCore, AppStatus, Clock, Config};

pub fn run() {
    hardening::apply();

    let app = tauri::Builder::default()
        .plugin(navigation_guard())
        .invoke_handler(tauri::generate_handler![
            commands::app_status,
            commands::activity,
            commands::create_vault,
            commands::unlock,
            commands::cancel_native_operation,
            commands::enable_quick_unlock,
            commands::disable_quick_unlock,
            commands::quick_unlock,
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
            commands::append_text,
            commands::show_field,
            commands::show_wallet_field,
            commands::show_and_copy_wallet_field,
            commands::hide_field,
            commands::extend_reveal,
            commands::copy_field,
            commands::copy_visible_field,
            commands::copy_field_line,
            commands::copy_field_env,
            commands::seed_wordlist,
            commands::check_seed,
            commands::check_private_key,
            commands::pick_backup_destination,
            commands::write_backup,
            commands::pick_backup_to_open,
            commands::inspect_backup,
            commands::replace_with_backup,
            commands::discard_inspection,
            commands::forget_backup_destination,
            commands::list_safety_copies,
            commands::inspect_safety_copy,
            commands::restore_safety_copy,
            commands::change_password,
            commands::set_settings,
            commands::mark_seed_explainer_seen,
            commands::vault_location,
            commands::dismiss_sharing_guard,
            commands::companion_status,
            commands::companion_start,
            commands::companion_stop,
            commands::companion_setup,
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
            // The Agent Companion ends with the app (ADR-0009).
            companion::stop_on_exit(app);
        }
    });
}

/// The window may load the app's own pages and nothing else.
///
/// The CSP stops the page from fetching, posting or embedding anything outside the app, but a
/// CSP has no say over the window itself going to another address, and a script could put
/// stolen values in such an address (ADR-0001). Tauri asks this before every navigation; its
/// own isolation frame is exempt before we are asked.
fn navigation_guard() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::<tauri::Wry>::new("navigation-guard")
        .on_navigation(|webview, url| {
            if webview
                .app_handle()
                .try_state::<native_coordinator::CoordinatorState>()
                .is_some()
            {
                native_coordinator::cancel(webview.app_handle());
                quick_runtime::cancel(webview.app_handle());
            }
            let dev_url = webview.app_handle().config().build.dev_url.as_ref();
            is_app_url(url, dev_url.filter(|_| cfg!(debug_assertions)))
        })
        .build()
}

/// The app's pages: `tauri://localhost` (Mac), `http(s)://tauri.localhost` (Windows) and, when
/// `dev_url` is given (debug builds only), the dev server. `about:blank` leads nowhere.
fn is_app_url(url: &Url, dev_url: Option<&Url>) -> bool {
    let plain_host = |host: &str| url.host_str() == Some(host) && url.port().is_none();
    match url.scheme() {
        "tauri" => plain_host("localhost"),
        "http" | "https" if plain_host("tauri.localhost") => true,
        "about" => url.as_str() == "about:blank",
        _ => dev_url.is_some_and(|dev| url.origin() == dev.origin()),
    }
}

fn setup(app: &AppHandle) {
    // Before anything else can be copied: a copy the last run left behind (crash, force quit).
    platform::clear_leftover_copy();
    let mut core = data_dir(app)
        .and_then(|data_dir| {
            let home_dir = match platform::real_home_dir() {
                Some(home) => home,
                None => app.path().home_dir().map_err(|e| e.to_string())?,
            };
            AppCore::new(Config {
                data_dir,
                home_dir,
                limits: vault_format::Limits::PRODUCTION,
                clock: Box::new(SystemClock::start()),
                clipboard: Box::new(platform::SystemClipboard),
                // Asked once: where the OS can't say when input was last seen (every
                // platform but the Mac, for now), "count activity anywhere" isn't offered.
                activity_anywhere_supported: platform::ms_since_last_input().is_some(),
            })
            .map_err(|e| e.message)
        })
        .unwrap_or_else(|message| fatal(&message));
    let biometry = biometry::availability();
    core.set_quick_unlock_availability(
        biometry == biometry::Availability::Available,
        match biometry {
            biometry::Availability::Available => None,
            biometry::Availability::UnsupportedPlatform => {
                Some("Touch ID is available only on supported Macs.")
            }
            biometry::Availability::BiometryUnavailable => {
                Some("Touch ID is not available on this Mac.")
            }
        },
    );
    core.set_lock_events_unavailable(platform::lock_events_unavailable());
    let quick_unlock_path = core
        .quick_unlock_path()
        .unwrap_or_else(|error| fatal(&error.message));
    app.manage(CoreState(Mutex::new(core)));
    app.manage(native_coordinator::CoordinatorState::default());
    app.manage(quick_runtime::QuickRuntime::new(quick_unlock_path));
    // Supervised beside the core, never inside it: the Vault core holds no helper state.
    app.manage(companion::Companion::new(companion::system_transport()));

    let handle = app.clone();
    platform::observe_lock_events(app, move |event| {
        // Sleep, screen lock, user switch and quit end any Agent Companion grant at once.
        if matches!(
            event,
            OsEvent::Sleep | OsEvent::ScreenLocked | OsEvent::UserSwitched | OsEvent::Quit
        ) {
            companion::stop_for_lifecycle(&handle);
        }
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
///
/// While the owner has "count activity anywhere" on, each round also asks the OS how long ago
/// the keyboard or mouse was last used in any app and tells the core. What that is worth is
/// decided by the session rules, not here.
fn background(app: &AppHandle) {
    let mut last: Option<AppStatus> = None;
    for round in 0u64.. {
        std::thread::sleep(Duration::from_millis(500));
        let probe = if round % 4 == 0 {
            probe_platform(app)
        } else {
            None
        };
        let mut core = commands::core(app);
        core.set_lock_events_unavailable(platform::lock_events_unavailable());
        if let Some((apps, capture_hiding_active)) = probe {
            // A list that couldn't be read says nothing: the Sharing Guard keeps what it knew.
            if let Some(apps) = apps {
                core.guard_update(&apps);
            }
            core.set_capture_hiding_active(capture_hiding_active);
        }
        if core.counts_activity_anywhere()
            && let Some(ago_ms) = platform::ms_since_last_input()
        {
            core.activity_elsewhere(ago_ms);
        }
        core.tick();
        native_coordinator::sync(app, &mut core);
        quick_runtime::sync(app, &mut core);
        let key = without_countdowns(core.app_status());
        // Whatever locked the Vault (Auto-lock, Lock, a lost focus, an OS event), an Agent
        // Companion grant ends with it (ADR-0009). Only an atomic check unless one may run.
        if key.phase != "unlocked" {
            companion::stop_for_lifecycle(app);
        }
        if last.as_ref() != Some(&key) {
            let _ = app.emit(STATUS_EVENT, commands::status(&core));
            last = Some(key);
        }
    }
}

/// Running apps and the self-check, read on the main thread (AppKit requires it for NSWindow).
fn probe_platform(app: &AppHandle) -> Option<(Option<Vec<guard::RunningApp>>, Option<bool>)> {
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

/// Session time since the app started. Where the platform has a clock that keeps counting
/// through sleep it is used, so idle time, shown values and copies also run out while the
/// computer sleeps; otherwise `Instant`.
struct SystemClock {
    started: Instant,
    started_continuous: Option<u64>,
}

impl SystemClock {
    fn start() -> SystemClock {
        SystemClock {
            started: Instant::now(),
            started_continuous: platform::continuous_ms(),
        }
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        match (self.started_continuous, platform::continuous_ms()) {
            (Some(started), Some(now)) => now.saturating_sub(started),
            _ => u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX),
        }
    }

    fn continuous_ms(&self) -> Option<u64> {
        platform::continuous_ms()
    }

    fn unix_secs(&self) -> i64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use super::{Url, is_app_url};

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a valid test URL")
    }

    #[test]
    fn only_the_apps_own_pages_may_be_loaded() {
        for own in [
            "tauri://localhost",
            "tauri://localhost/",
            "tauri://localhost/index.html",
            "http://tauri.localhost/",
            "https://tauri.localhost/assets/index.js",
            "about:blank",
        ] {
            assert!(is_app_url(&url(own), None), "{own}");
        }
        for other in [
            "https://example.com/",
            "https://example.com/c?d=fake-value",
            "http://127.0.0.1:1420/",
            "http://localhost/",
            "https://localhost/",
            "tauri://example.com/",
            "tauri://localhost:8080/",
            "http://tauri.localhost:8080/",
            "https://tauri.localhost.example.com/",
            "https://user@tauri.localhost.example.com/",
            "file:///etc/hosts",
            "data:text/html,fake",
            "javascript:void(0)",
            "about:srcdoc",
            "blob:https://example.com/fake",
            "isolation://localhost/",
        ] {
            assert!(!is_app_url(&url(other), None), "{other}");
        }
    }

    #[test]
    fn the_dev_server_is_allowed_only_when_given() {
        let dev = url("http://127.0.0.1:1420");
        assert!(is_app_url(&url("http://127.0.0.1:1420/"), Some(&dev)));
        assert!(is_app_url(
            &url("http://127.0.0.1:1420/src/main.tsx"),
            Some(&dev)
        ));
        assert!(is_app_url(&url("tauri://localhost/"), Some(&dev)));
        for other in [
            "http://127.0.0.1:1421/",
            "https://127.0.0.1:1420/",
            "http://localhost:1420/",
            "https://example.com/",
        ] {
            assert!(!is_app_url(&url(other), Some(&dev)), "{other}");
        }
    }
}
