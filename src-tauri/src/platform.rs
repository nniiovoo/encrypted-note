//! OS adapters: thin wrappers around platform APIs, one set per OS (ADR-0002, PRD module 6).
//!
//! * [`capture_hiding_active`]: the Capture Hiding self-check (main thread only).
//! * [`running_apps`]: what the Sharing Guard classifies.
//! * [`SystemClipboard`]: the clipboard, with our copies marked private (kept out of OS history
//!   and cloud sync; clipboard managers skip them only by convention). It never reads the
//!   clipboard's contents.
//! * [`observe_lock_events`]: OS notifications that must lock the app at once, and app switches.
//! * [`lock_events_unavailable`]: Windows lock notifications whose registration failed.
//! * [`default_backup_folder`]: where the Backup save panel starts.
//! * [`clear_leftover_copy`]: at startup, clear a copy the last run didn't live to clear.
//! * [`real_home_dir`]: the owner's home folder, even inside the Mac App Sandbox.
//! * [`continuous_ms`]: a monotonic clock that keeps counting while the computer sleeps.
//! * [`ms_since_last_input`]: how long ago the keyboard, mouse or trackpad was last used in
//!   any app, for the opt-in "count activity anywhere" Auto-lock setting (macOS only). A key
//!   that is merely held down does not count as use.
//!
//! macOS and Windows are implemented here; other platforms get harmless stand-ins (no self-check,
//! no running apps, no copy).
//!
//! This module, `hardening.rs` and `wipe_on_free.rs` are the only places allowed to use
//! `unsafe`; every block says why it is sound.

#[cfg(target_os = "macos")]
pub use mac::*;
#[cfg(not(any(target_os = "macos", windows)))]
pub use other::*;
#[cfg(windows)]
pub use win::*;

#[cfg(target_os = "macos")]
mod mac {
    use std::path::{Path, PathBuf};
    use std::ptr::NonNull;

    use block2::RcBlock;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{
        NSApplicationDidResignActiveNotification, NSPasteboard, NSPasteboardContentsOptions,
        NSPasteboardTypeString, NSWindow, NSWorkspace, NSWorkspaceScreensDidSleepNotification,
        NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_core_foundation::{CFArray, CFDictionary, CFNumber, CFRetained, CFString, CFType};
    use objc2_core_graphics::{
        CGWindowListCopyWindowInfo, CGWindowListOption, CGWindowSharingType, kCGWindowSharingState,
    };
    use objc2_foundation::{
        NSData, NSDistributedNotificationCenter, NSNotification, NSNotificationCenter, NSString,
        ns_string,
    };
    use session::OsEvent;
    use tauri::{AppHandle, Runtime, WebviewWindow};

    use crate::core::{ApiError, Clipboard};

    /// Does the window server report our window's `kCGWindowSharingState` as
    /// `kCGWindowSharingNone` (ADR-0002)? That is what the window server applies, not just the
    /// `sharingType` flag tao set. `None` off the main thread or if the window can't be read.
    pub fn capture_hiding_active<R: Runtime>(window: &WebviewWindow<R>) -> Option<bool> {
        MainThreadMarker::new()?;
        let ns_window = window.ns_window().ok()?.cast::<NSWindow>();
        // SAFETY: `ns_window()` returns tao's pointer to this live NSWindow, which `window` keeps
        // alive for the duration of this borrow; we checked above that we're on the main thread,
        // where AppKit requires NSWindow to be used; `windowNumber` only reads a property.
        let number = u32::try_from(unsafe { ns_window.as_ref() }?.windowNumber()).ok()?;
        let windows =
            CGWindowListCopyWindowInfo(CGWindowListOption::OptionIncludingWindow, number)?;
        // SAFETY: CGWindowListCopyWindowInfo returns a CFArray of CFDictionary with CFString keys
        // (Apple's "Required Window List Keys"); values are checked with `downcast` below.
        let windows = unsafe {
            CFRetained::cast_unchecked::<CFArray<CFDictionary<CFString, CFType>>>(windows)
        };
        // SAFETY: an immutable CFString constant exported by CoreGraphics, valid for the process.
        let key = unsafe { kCGWindowSharingState };
        let state = windows
            .get(0)?
            .get(key)?
            .downcast::<CFNumber>()
            .ok()?
            .as_i64()?;
        Some(state == i64::from(CGWindowSharingType::None.0))
    }

    /// Bundle ids of the running applications. `None` if they could not be listed (never, here).
    pub fn running_apps() -> Option<Vec<guard::RunningApp>> {
        let apps = NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .map(|app| guard::RunningApp {
                bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
                exe_name: None,
            })
            .collect();
        Some(apps)
    }

    /// Our own pasteboard type, put next to every copy: it says "encrypted-note wrote this"
    /// without anyone having to read what was written.
    fn copy_marker() -> &'static NSString {
        ns_string!("com.encryptednote.app.copy")
    }

    /// If the clipboard still holds a copy from a run that ended without clearing it (a crash, a
    /// force quit, a power cut within the 30 s), clear it now. Only our marker is looked at,
    /// never the contents.
    pub fn clear_leftover_copy() {
        let pasteboard = NSPasteboard::generalPasteboard();
        if pasteboard.dataForType(copy_marker()).is_some() {
            pasteboard.clearContents();
        }
    }

    /// The owner's home folder from the user database. Inside the App Sandbox `$HOME` (which is
    /// what Tauri reads) is the app's container, while file panels return real paths.
    pub fn real_home_dir() -> Option<PathBuf> {
        use std::ffi::{CStr, OsStr};
        use std::os::unix::ffi::OsStrExt;
        let mut buffer = vec![0 as libc::c_char; 16 * 1024];
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut found: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: `entry`, `buffer` (with its true length) and `found` are live, writable locals
        // of the types getpwuid_r expects; it writes only into them.
        let status = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                entry.as_mut_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            )
        };
        if status != 0 || found.is_null() {
            return None;
        }
        // SAFETY: on success `found` points at `entry`, now initialised, whose `pw_dir` is a
        // NUL-terminated string inside `buffer`; both are still alive here.
        let dir = unsafe {
            let dir = (*found).pw_dir;
            if dir.is_null() {
                return None;
            }
            CStr::from_ptr(dir)
        };
        let home = PathBuf::from(OsStr::from_bytes(dir.to_bytes()));
        home.is_absolute().then_some(home)
    }

    /// Milliseconds on a clock that never goes back and keeps counting while the Mac sleeps.
    /// `Instant` stops during sleep here, so a Vault left Unlocked would wake with its whole
    /// idle time still to run if the sleep notification was ever missed.
    pub fn continuous_ms() -> Option<u64> {
        let mut now = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `now` is a live, writable timespec.
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) } != 0 {
            return None;
        }
        let secs = u64::try_from(now.tv_sec).ok()?;
        let nanos = u64::try_from(now.tv_nsec).ok()?;
        Some(secs.saturating_mul(1000).saturating_add(nanos / 1_000_000))
    }

    // CoreGraphics is already linked (the window list above comes from it). This one function
    // is declared here because the `CGEventSource` part of objc2-core-graphics is not among
    // the features this app turns on, and turning on a feature would change the dependency set.
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        /// `CFTimeInterval CGEventSourceSecondsSinceLastEventType(CGEventSourceStateID,
        /// CGEventType)`: an `int32_t` enum, a `uint32_t` enum, a `double` in seconds.
        fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
    }

    /// `kCGEventSourceStateHIDSystemState`: input from the hardware, whichever app it went to.
    const HID_SYSTEM_STATE: i32 = 1;
    /// The `CGEventType`s that count as someone using the computer: a mouse button going down
    /// or up (left 1 and 2, right 3 and 4, any other 25 and 26), the pointer moving (5) or
    /// dragging (6, 7, 27), a key coming up (11), a modifier key changing (12), scrolling (22).
    ///
    /// Key down (10) is left out on purpose, and so is `kCGAnyInputEventType`, which includes
    /// it: a key that is merely held down sends key-downs for as long as it is held, and a
    /// stuck or leaned-on key must not keep Auto-lock away (the app's own window ignores key
    /// auto-repeat for the same reason). Typing still counts, through the key-up that ends each
    /// stroke. Whatever else is left out (a tablet pen, a touch on the trackpad that moves
    /// nothing) can only make the app lock sooner.
    const INPUT_EVENTS: [u32; 13] = [1, 2, 3, 4, 5, 6, 7, 11, 12, 22, 25, 26, 27];

    /// Milliseconds since the keyboard, mouse or trackpad was last used, in any app: the most
    /// recent of [`INPUT_EVENTS`], so a key that is merely held down is not use. It reads
    /// numbers the window server keeps: no event is observed and nothing typed is seen.
    /// `None` if no answer is a usable time.
    pub fn ms_since_last_input() -> Option<u64> {
        INPUT_EVENTS
            .iter()
            .filter_map(|&event_type| ms_since_last(event_type))
            .min()
    }

    /// Milliseconds since the hardware last sent an event of this one `CGEventType` (a long
    /// time, not an error, for a type never seen). `None` if the answer isn't a usable time.
    fn ms_since_last(event_type: u32) -> Option<u64> {
        // SAFETY: a plain C function taking two integers by value and returning a double; it
        // reads a counter and has no preconditions. The state is the documented constant, and
        // any event type is accepted.
        let seconds =
            unsafe { CGEventSourceSecondsSinceLastEventType(HID_SYSTEM_STATE, event_type) };
        // The cast saturates, and so does the multiplication: far past any idle time is fine.
        (seconds.is_finite() && seconds >= 0.0).then_some((seconds * 1000.0) as u64)
    }

    /// The general pasteboard, written current-host-only (no Universal Clipboard) and marked
    /// Concealed + Transient (nspasteboard.org), which well-behaved clipboard managers and
    /// history tools skip by convention (macOS doesn't enforce it).
    pub struct SystemClipboard;

    impl Clipboard for SystemClipboard {
        fn write(&mut self, text: &str) -> Result<u64, ApiError> {
            let pasteboard = NSPasteboard::generalPasteboard();
            pasteboard
                .prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
            // SAFETY: an immutable NSString constant exported by AppKit, valid for the whole process.
            let string_type = unsafe { NSPasteboardTypeString };
            let marker = NSData::new();
            // Markers first, the text last: a clipboard manager that looks the moment the
            // pasteboard changes must never find the text without them. (`NSString::from_str`
            // makes a copy AppKit owns; it can't be wiped from here.)
            let written = pasteboard
                .setData_forType(Some(&marker), ns_string!("org.nspasteboard.ConcealedType"))
                && pasteboard
                    .setData_forType(Some(&marker), ns_string!("org.nspasteboard.TransientType"))
                && pasteboard.setData_forType(Some(&marker), copy_marker())
                && pasteboard.setString_forType(&NSString::from_str(text), string_type);
            if !written {
                pasteboard.clearContents();
                return Err(ApiError {
                    code: "io",
                    message: "The clipboard didn't accept the copy. Try again.".to_owned(),
                });
            }
            Ok(change_count(&pasteboard))
        }

        fn clear_if(&mut self, count: u64) -> bool {
            let pasteboard = NSPasteboard::generalPasteboard();
            if change_count(&pasteboard) == count {
                pasteboard.clearContents();
            }
            true
        }
    }

    fn change_count(pasteboard: &NSPasteboard) -> u64 {
        u64::try_from(pasteboard.changeCount()).unwrap_or(0)
    }

    /// Call `on_event` for sleep, display sleep (lid close), user switch and screen lock, and
    /// `FocusLost` when the app stops being the active app. That one also fires while our own
    /// Backup file dialog has focus, when window focus events are ignored. Must be called on the
    /// main thread; the observations last for the life of the process.
    pub fn observe_lock_events(
        _app: &AppHandle,
        on_event: impl Fn(OsEvent) + Clone + Send + Sync + 'static,
    ) {
        let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
        // SAFETY: immutable NSString constants exported by AppKit, valid for the whole process.
        let workspace_events = unsafe {
            [
                (NSWorkspaceWillSleepNotification, OsEvent::Sleep),
                (
                    NSWorkspaceScreensDidSleepNotification,
                    OsEvent::ScreenLocked,
                ),
                (
                    NSWorkspaceSessionDidResignActiveNotification,
                    OsEvent::UserSwitched,
                ),
            ]
        };
        for (name, event) in workspace_events {
            observe(&workspace, name, event, on_event.clone());
        }
        observe(
            &NSNotificationCenter::defaultCenter(),
            // SAFETY: an immutable NSString constant exported by AppKit, valid for the whole process.
            unsafe { NSApplicationDidResignActiveNotification },
            OsEvent::FocusLost,
            on_event.clone(),
        );
        let distributed = NSDistributedNotificationCenter::defaultCenter();
        observe(
            &distributed,
            ns_string!("com.apple.screenIsLocked"),
            OsEvent::ScreenLocked,
            on_event,
        );
    }

    fn observe(
        center: &NSNotificationCenter,
        name: &NSString,
        event: OsEvent,
        on_event: impl Fn(OsEvent) + Send + Sync + 'static,
    ) {
        let block = RcBlock::new(move |_: NonNull<NSNotification>| on_event(event));
        // SAFETY: `obj` is None (any sender), so there is no type to get wrong; `queue` is None, so
        // the block runs on the posting thread, and it is Send + Sync + 'static (it only hands the
        // event to `on_event`, which may run on any thread). The returned observer token is leaked
        // on purpose: the observation must last until the process exits.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        std::mem::forget(token);
    }

    /// A removable volume (USB stick, SD card) if one is mounted, else the home folder. Never
    /// Desktop or Documents (they may sync to iCloud).
    pub fn default_backup_folder(home: &Path) -> PathBuf {
        external_volume(Path::new("/Volumes")).unwrap_or_else(|| home.to_path_buf())
    }

    /// The first real mount point under `/Volumes`: a directory (not the symlink to the startup
    /// disk) on a different device from `/`.
    fn external_volume(volumes: &Path) -> Option<PathBuf> {
        use std::os::unix::fs::MetadataExt;
        let root = std::fs::metadata("/").ok()?.dev();
        std::fs::read_dir(volumes)
            .ok()?
            .flatten()
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .map(|entry| entry.path())
            .find(|path| {
                std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.dev() != root)
            })
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn the_time_since_the_last_input_can_be_read() {
            // On a Mac nobody is touching this is a long time, but it is a time: the call
            // links, and what it returns is neither negative nor not-a-number.
            assert!(super::ms_since_last_input().is_some());
        }

        /// `kCGEventKeyDown`: what a held key keeps sending.
        const KEY_DOWN: u32 = 10;
        /// `kCGEventKeyUp`: what ends every key stroke.
        const KEY_UP: u32 = 11;
        /// `kCGAnyInputEventType`: every kind of input, key-downs included.
        const ANY_INPUT: u32 = u32::MAX;

        #[test]
        fn a_key_that_is_held_down_is_not_input_that_counts() {
            assert!(!super::INPUT_EVENTS.contains(&KEY_DOWN));
            assert!(!super::INPUT_EVENTS.contains(&ANY_INPUT));
            // Typing still counts.
            assert!(super::INPUT_EVENTS.contains(&KEY_UP));
            // Every kind asked about gives a time, also one this Mac has never seen.
            for event_type in super::INPUT_EVENTS {
                assert!(super::ms_since_last(event_type).is_some(), "{event_type}");
            }
        }

        #[test]
        fn leaving_input_out_never_makes_the_computer_look_more_recently_used() {
            // Ours is read first: input that arrives between the two reads can then only make
            // the any-input time shorter. A kind of input that answered "just now" without
            // having happened would keep Auto-lock away for as long as the rules allow; this
            // is the check for that.
            let ours = super::ms_since_last_input().unwrap();
            let any = super::ms_since_last(ANY_INPUT).unwrap();
            assert!(any <= ours.saturating_add(1_000), "{any} > {ours}");
        }
    }
}

#[cfg(not(windows))]
pub fn lock_events_unavailable() -> Vec<&'static str> {
    Vec::new()
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq)]
enum LockRegistration {
    WindowHook = 1,
    Session = 2,
    DisplayOff = 4,
    LidClose = 8,
}

#[cfg(any(windows, test))]
fn register_lock_notifications(mut register: impl FnMut(LockRegistration) -> bool) -> u8 {
    let mut registered = 0;
    for kind in [
        LockRegistration::WindowHook,
        LockRegistration::Session,
        LockRegistration::DisplayOff,
        LockRegistration::LidClose,
    ] {
        if register(kind) {
            registered |= kind as u8;
        } else if kind == LockRegistration::WindowHook {
            break;
        }
    }
    registered
}

#[cfg(any(windows, test))]
fn missing_lock_events(registered: u8) -> Vec<&'static str> {
    use LockRegistration::*;
    let hook = WindowHook as u8;
    [
        ("sleep", hook),
        ("screen_lock", hook | Session as u8),
        ("user_switch", hook | Session as u8),
        ("display_off", hook | DisplayOff as u8),
        ("lid_close", hook | LidClose as u8),
    ]
    .into_iter()
    .filter(|(_, required)| registered & required != *required)
    .map(|(event, _)| event)
    .collect()
}

#[cfg(any(windows, test))]
fn unregister_lock_notifications(
    registrations: &std::sync::atomic::AtomicU8,
    mut unregister: impl FnMut(LockRegistration),
) {
    let registered = registrations.swap(0, std::sync::atomic::Ordering::SeqCst);
    // The window hook stays until WM_NCDESTROY, after these notification registrations end.
    for kind in [
        LockRegistration::Session,
        LockRegistration::DisplayOff,
        LockRegistration::LidClose,
    ] {
        if registered & kind as u8 != 0 {
            unregister(kind);
        }
    }
}

#[cfg(test)]
mod lock_registration_tests {
    use super::{
        LockRegistration::*, missing_lock_events, register_lock_notifications,
        unregister_lock_notifications,
    };
    use std::sync::atomic::{AtomicU8, Ordering};

    #[test]
    fn startup_reports_unavailable_until_registered() {
        assert_eq!(
            missing_lock_events(0),
            [
                "sleep",
                "screen_lock",
                "user_switch",
                "display_off",
                "lid_close"
            ]
        );
    }

    #[test]
    fn failed_window_hook_stops_registration_and_reports_every_event() {
        let mut called = Vec::new();
        let registered = register_lock_notifications(|kind| {
            called.push(kind);
            false
        });
        assert_eq!(called, [WindowHook]);
        assert_eq!(missing_lock_events(registered), missing_lock_events(0));
    }

    #[test]
    fn partial_failures_report_only_the_affected_events() {
        for (failed, missing) in [
            (Session, vec!["screen_lock", "user_switch"]),
            (DisplayOff, vec!["display_off"]),
            (LidClose, vec!["lid_close"]),
        ] {
            let registered = register_lock_notifications(|kind| kind != failed);
            assert_eq!(missing_lock_events(registered), missing);
        }
        assert!(missing_lock_events(register_lock_notifications(|_| true)).is_empty());
    }

    #[test]
    fn successful_session_retry_clears_its_warning() {
        let registrations = AtomicU8::new(register_lock_notifications(|kind| kind != Session));
        assert_eq!(
            missing_lock_events(registrations.load(Ordering::SeqCst)),
            ["screen_lock", "user_switch"]
        );
        registrations.fetch_or(Session as u8, Ordering::SeqCst);
        assert!(missing_lock_events(registrations.load(Ordering::SeqCst)).is_empty());
    }

    #[test]
    fn cleanup_unregisters_only_successes_once_and_clears_health() {
        let registrations = AtomicU8::new(register_lock_notifications(|kind| kind != DisplayOff));
        let mut removed = Vec::new();
        unregister_lock_notifications(&registrations, |kind| removed.push(kind));
        unregister_lock_notifications(&registrations, |_| panic!("already unregistered"));
        assert_eq!(removed, [Session, LidClose]);
        assert_eq!(
            missing_lock_events(registrations.load(Ordering::SeqCst)),
            missing_lock_events(0)
        );
    }
}

#[cfg(windows)]
mod win {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU8, Ordering};
    use std::time::Duration;

    use session::OsEvent;
    use tauri::{AppHandle, Manager, Runtime, WebviewWindow};
    use windows::Win32::Foundation::{
        COLORREF, CloseHandle, GlobalFree, HANDLE, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EmptyClipboard, GetClipboardOwner, GetClipboardSequenceNumber,
        OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Memory::{
        GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock,
    };
    use windows::Win32::System::Power::{
        HPOWERNOTIFY, POWERBROADCAST_SETTING, RegisterPowerSettingNotification,
        UnregisterPowerSettingNotification,
    };
    use windows::Win32::System::RemoteDesktop::{
        NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
    };
    use windows::Win32::System::SystemServices::{
        GUID_CONSOLE_DISPLAY_STATE, GUID_LIDSWITCH_STATE_CHANGE,
    };
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        DEVICE_NOTIFY_WINDOW_HANDLE, GWL_EXSTYLE, GetForegroundWindow, GetWindowDisplayAffinity,
        GetWindowLongW, GetWindowThreadProcessId, KillTimer, LWA_ALPHA, PBT_APMSUSPEND,
        PBT_POWERSETTINGCHANGE, SIZE_MAXIMIZED, SIZE_MINIMIZED, SIZE_RESTORED, STYLESTRUCT,
        SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetTimer, SetWindowDisplayAffinity,
        SetWindowLongW, WDA_EXCLUDEFROMCAPTURE, WINDOWPOS, WM_DESTROY, WM_NCDESTROY,
        WM_POWERBROADCAST, WM_SIZE, WM_STYLECHANGING, WM_TIMER, WM_WINDOWPOSCHANGED,
        WM_WTSSESSION_CHANGE, WS_EX_LAYERED, WTS_CONSOLE_DISCONNECT, WTS_REMOTE_DISCONNECT,
        WTS_SESSION_LOCK, WTS_SESSION_LOGOFF,
    };
    use windows::core::{PCWSTR, w};

    use crate::core::{ApiError, Clipboard};

    /// The main window's handle. Tauri's `HWND` comes from its own copy of the `windows` crate;
    /// the raw handle is the same.
    fn hwnd<R: Runtime>(window: &WebviewWindow<R>) -> Option<HWND> {
        window.hwnd().ok().map(|hwnd| HWND(hwnd.0))
    }

    /// Is our window's display affinity `WDA_EXCLUDEFROMCAPTURE`, with the `WS_EX_LAYERED`
    /// workaround (see [`apply_capture_hiding`]) in place (ADR-0002)? tao ignores the result of
    /// the set call, so this reads back what Windows applied. `None` if it can't tell.
    ///
    /// What this can't see: Windows can draw the window into captures as a black box while still
    /// reporting `WDA_EXCLUDEFROMCAPTURE` (the hide-then-show bug, Tauri #14189). Before Windows 10
    /// 2004 the value acts as `WDA_MONITOR` (also a black box) and may still read back as set. In
    /// both cases the content stays hidden, but "excluded" is claimed when it isn't.
    pub fn capture_hiding_active<R: Runtime>(window: &WebviewWindow<R>) -> Option<bool> {
        let hwnd = hwnd(window)?;
        let mut affinity = 0;
        // SAFETY: the out-pointer is a live local u32; user32 validates the window handle, so
        // even a stale one only makes the call fail.
        unsafe { GetWindowDisplayAffinity(hwnd, &mut affinity) }.ok()?;
        Some(affinity == WDA_EXCLUDEFROMCAPTURE.0 && ex_style(hwnd) & WS_EX_LAYERED.0 != 0)
    }

    /// The window's extended style bits (0 if the handle is stale).
    fn ex_style(hwnd: HWND) -> u32 {
        // SAFETY: plain call; user32 validates the handle.
        unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 }
    }

    /// Executable file names of the running processes (e.g. "Zoom.exe"). `None` if the list
    /// could not be read: that says nothing about what is running, so the Sharing Guard keeps
    /// what it knew.
    pub fn running_apps() -> Option<Vec<guard::RunningApp>> {
        let mut apps = Vec::new();
        // SAFETY: plain call with no pointers; the snapshot handle is closed below.
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return None;
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: `snapshot` is the live snapshot handle and `entry` a writable PROCESSENTRY32W
        // with `dwSize` set, as Process32FirstW requires.
        let mut more = unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok();
        while more {
            let name = &entry.szExeFile;
            let len = name.iter().position(|&c| c == 0).unwrap_or(name.len());
            apps.push(guard::RunningApp {
                bundle_id: None,
                exe_name: Some(String::from_utf16_lossy(&name[..len])),
            });
            // SAFETY: as for Process32FirstW above.
            more = unsafe { Process32NextW(snapshot, &mut entry) }.is_ok();
        }
        // SAFETY: `snapshot` is our handle, closed exactly once.
        let _ = unsafe { CloseHandle(snapshot) };
        Some(apps)
    }

    /// Not done on Windows yet: a copy left on the clipboard by a run that crashed stays until
    /// something replaces it. (The Mac adapter clears it at the next start.)
    pub fn clear_leftover_copy() {}

    /// Tauri's home folder is the real one on Windows.
    pub fn real_home_dir() -> Option<PathBuf> {
        None
    }

    /// `Instant` already keeps counting through sleep on Windows.
    pub fn continuous_ms() -> Option<u64> {
        None
    }

    /// Not done on Windows yet: "count activity anywhere" is offered on the Mac only.
    pub fn ms_since_last_input() -> Option<u64> {
        None
    }

    /// `CF_UNICODETEXT`, defined in the OLE bindings this crate doesn't otherwise need.
    const CF_UNICODETEXT: u32 = 13;

    /// Windows' private-data clipboard formats, each written as a DWORD 0. Windows keeps the copy
    /// out of Win+V history and the cloud clipboard; well-behaved third-party clipboard monitors
    /// also skip it, by convention (Windows doesn't enforce that).
    const PRIVATE_FORMATS: [PCWSTR; 3] = [
        w!("ExcludeClipboardContentFromMonitorProcessing"),
        w!("CanIncludeInClipboardHistory"),
        w!("CanUploadToCloudClipboard"),
    ];

    /// `write`'s change count when the clipboard may no longer hold our copy: sequence numbers
    /// are u32, so `clear_if` never matches it and leaves the clipboard alone.
    const NOT_OUR_COPY: u64 = u64::MAX;

    /// The main window, which owns the clipboard while it holds our copy (raw `HWND`, 0 until
    /// [`observe_lock_events`] runs).
    static CLIPBOARD_OWNER: AtomicIsize = AtomicIsize::new(0);

    fn clipboard_owner() -> Option<HWND> {
        match CLIPBOARD_OWNER.load(Ordering::SeqCst) {
            0 => None,
            raw => Some(HWND(raw as *mut _)),
        }
    }

    /// The system clipboard, with the private-data formats above next to the text.
    pub struct SystemClipboard;

    impl Clipboard for SystemClipboard {
        fn write(&mut self, text: &str) -> Result<u64, ApiError> {
            let failed = || ApiError {
                code: "io",
                message: "The clipboard didn't accept the copy. Try again.".to_owned(),
            };
            // SAFETY: the names are static NUL-terminated UTF-16 strings from `w!`.
            let formats = PRIVATE_FORMATS.map(|name| unsafe { RegisterClipboardFormatW(name) });
            if formats.contains(&0) {
                return Err(failed());
            }
            // Opened with an owner window: after EmptyClipboard, SetClipboardData is documented to
            // fail when the clipboard was opened without one.
            let owner = clipboard_owner().ok_or_else(failed)?;
            {
                let _open = OpenedClipboard::open(Some(owner)).ok_or_else(failed)?;
                let text_bytes = (text.encode_utf16().count() + 1) * 2;
                // SAFETY: the clipboard is open (`_open`).
                let written = unsafe { EmptyClipboard() }.is_ok()
                    && formats.iter().all(|&format| set_data(format, 4, |_| {}))
                    && set_data(CF_UNICODETEXT, text_bytes, |bytes| {
                        for (unit, out) in text.encode_utf16().zip(bytes.chunks_exact_mut(2)) {
                            out.copy_from_slice(&unit.to_ne_bytes());
                        }
                    });
                if !written {
                    // Never leave the text without its private markers, or half a copy.
                    // SAFETY: the clipboard is still open.
                    let _ = unsafe { EmptyClipboard() };
                    return Err(failed());
                }
            }
            // Read once the clipboard is closed, so the count covers what Windows adds on close
            // (CF_LOCALE). Another app that wrote since has emptied the clipboard, which made it the
            // owner: then the count may be theirs, and `clear_if` must never wipe their data.
            // SAFETY: plain calls with no arguments.
            let (count, owner_now) = unsafe { (GetClipboardSequenceNumber(), GetClipboardOwner()) };
            Ok(if owner_now.is_ok_and(|now| now == owner) {
                u64::from(count)
            } else {
                NOT_OUR_COPY
            })
        }

        fn clear_if(&mut self, count: u64) -> bool {
            // No owner window: this may run at exit, after the main window is gone.
            let Some(_open) = OpenedClipboard::open(None) else {
                return false;
            };
            // Compared while we hold the clipboard, so nobody can write in between.
            // SAFETY: plain calls; the clipboard is open (`_open`).
            unsafe {
                if u64::from(GetClipboardSequenceNumber()) != count {
                    true
                } else {
                    EmptyClipboard().is_ok()
                }
            }
        }
    }

    /// Put `len` zeroed bytes, filled in by `fill`, on the open clipboard as `format`.
    fn set_data(format: u32, len: usize, fill: impl FnOnce(&mut [u8])) -> bool {
        // SAFETY: plain allocation; on success the clipboard owns it, otherwise it's freed below.
        let Ok(memory) = (unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, len) }) else {
            return false;
        };
        // SAFETY: `memory` is a live block of `len` zeroed bytes that only we can see; while it is
        // locked, the pointer is valid for exactly those bytes.
        let locked = unsafe {
            let data = GlobalLock(memory).cast::<u8>();
            if !data.is_null() {
                fill(std::slice::from_raw_parts_mut(data, len));
                let _ = GlobalUnlock(memory);
            }
            !data.is_null()
        };
        // SAFETY: the caller holds the clipboard open; `memory` is an unlocked GMEM_MOVEABLE block,
        // as SetClipboardData requires.
        if locked && unsafe { SetClipboardData(format, Some(HANDLE(memory.0))) }.is_ok() {
            return true;
        }
        // SAFETY: the clipboard didn't take `memory`, so it is still ours: `len` bytes, valid
        // while locked. Wipe what `fill` wrote (it may be the copied value), then free it once.
        unsafe {
            let data = GlobalLock(memory).cast::<u8>();
            if !data.is_null() {
                std::ptr::write_bytes(data, 0, len);
                let _ = GlobalUnlock(memory);
            }
            let _ = GlobalFree(Some(memory));
        }
        false
    }

    /// The clipboard, opened by us and closed on drop.
    struct OpenedClipboard;

    impl OpenedClipboard {
        /// Other apps hold the clipboard briefly while they read or write it: retry for ~100 ms.
        /// `owner` becomes the clipboard owner if we then empty it. We never use delayed
        /// rendering, so it never has to answer WM_RENDERFORMAT.
        fn open(owner: Option<HWND>) -> Option<Self> {
            for _ in 0..10 {
                // SAFETY: plain call; user32 validates the handle, so a stale one only fails.
                if unsafe { OpenClipboard(owner) }.is_ok() {
                    return Some(Self);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            None
        }
    }

    impl Drop for OpenedClipboard {
        fn drop(&mut self) {
            // SAFETY: we opened the clipboard on this thread and close it once.
            let _ = unsafe { CloseClipboard() };
        }
    }

    static ON_EVENT: OnceLock<Box<dyn Fn(OsEvent) + Send + Sync>> = OnceLock::new();
    static LOCK_REGISTRATIONS: AtomicU8 = AtomicU8::new(0);

    pub fn lock_events_unavailable() -> Vec<&'static str> {
        super::missing_lock_events(LOCK_REGISTRATIONS.load(Ordering::SeqCst))
    }
    /// Stops the foreground-process watcher when the main window is destroyed.
    static FOREGROUND_RUNNING: AtomicBool = AtomicBool::new(false);
    /// Set while the main window is minimized, so restoring it re-applies Capture Hiding once.
    static MINIMIZED: AtomicBool = AtomicBool::new(false);
    /// Set once Capture Hiding is applied, so `WS_EX_LAYERED` survives tao's style rewrites.
    static CAPTURE_HIDING: AtomicBool = AtomicBool::new(false);
    /// The display-state registration's `HPOWERNOTIFY` (0 if none), undone in WM_DESTROY.
    static POWER_NOTIFY: AtomicIsize = AtomicIsize::new(0);
    /// The lid-state registration's `HPOWERNOTIFY` (0 if none), undone in WM_DESTROY.
    static LID_NOTIFY: AtomicIsize = AtomicIsize::new(0);
    const SUBCLASS_ID: usize = 1;
    /// Our WM_TIMER id for retrying the session registration; distinct from tao's timer ids.
    const SESSION_RETRY_TIMER: u32 = 0x454E_4F54;
    const SESSION_RETRY_MS: u32 = 2000;

    /// Observe OS lock events on the main window's thread, and report failed registrations in
    /// lock_events_unavailable. Session registration is retried while Terminal Services starts.
    pub fn observe_lock_events(
        app: &AppHandle,
        on_event: impl Fn(OsEvent) + Clone + Send + Sync + 'static,
    ) {
        use super::LockRegistration::*;
        let Some(hwnd) = app.get_webview_window("main").as_ref().and_then(hwnd) else {
            return;
        };
        if ON_EVENT.get().is_some() {
            return;
        }
        CLIPBOARD_OWNER.store(hwnd.0 as isize, Ordering::SeqCst);
        apply_capture_hiding(hwnd);
        let registered = super::register_lock_notifications(|kind| match kind {
            WindowHook => {
                // SAFETY: we're on the thread owning hwnd; window_proc has SUBCLASSPROC's signature.
                let installed =
                    unsafe { SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0) }.as_bool();
                if installed {
                    let _ = ON_EVENT.set(Box::new(on_event.clone()));
                } else {
                    eprintln!(
                        "encrypted-note: can't subclass the main window; OS events won't lock the app"
                    );
                }
                installed
            }
            Session => {
                let registered = register_session_notification(hwnd);
                if !registered {
                    eprintln!(
                        "encrypted-note: session notifications unavailable; retrying every {SESSION_RETRY_MS} ms"
                    );
                    // SAFETY: our timer on the window's own thread; it dies with the window.
                    if unsafe {
                        SetTimer(
                            Some(hwnd),
                            SESSION_RETRY_TIMER as usize,
                            SESSION_RETRY_MS,
                            None,
                        )
                    } == 0
                    {
                        eprintln!("encrypted-note: can't schedule the session-notification retry");
                    }
                }
                registered
            }
            DisplayOff | LidClose => {
                let (guid, slot, name) = if kind == DisplayOff {
                    (&GUID_CONSOLE_DISPLAY_STATE, &POWER_NOTIFY, "display-off")
                } else {
                    (&GUID_LIDSWITCH_STATE_CHANGE, &LID_NOTIFY, "lid-close")
                };
                // SAFETY: hwnd receives these notifications; successful handles are unregistered in WM_DESTROY.
                match unsafe {
                    RegisterPowerSettingNotification(
                        HANDLE(hwnd.0),
                        guid,
                        DEVICE_NOTIFY_WINDOW_HANDLE,
                    )
                } {
                    Ok(handle) => {
                        slot.store(handle.0, Ordering::SeqCst);
                        true
                    }
                    Err(error) => {
                        eprintln!("encrypted-note: {name} notifications unavailable ({error})");
                        false
                    }
                }
            }
        });
        LOCK_REGISTRATIONS.store(registered, Ordering::SeqCst);
        if registered & WindowHook as u8 == 0 {
            return;
        }
        if !FOREGROUND_RUNNING.swap(true, Ordering::SeqCst) {
            let process = std::process::id();
            std::thread::spawn(move || {
                let mut active = foreground_process_is(process);
                while FOREGROUND_RUNNING.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(200));
                    let next = foreground_process_is(process);
                    if active && !next {
                        notify(OsEvent::FocusLost);
                    } else if !active && next {
                        notify(OsEvent::FocusGained);
                    }
                    active = next;
                }
            });
        }
    }

    /// Screen lock, user switch, disconnect and log off, delivered to `hwnd`.
    fn register_session_notification(hwnd: HWND) -> bool {
        // SAFETY: only names `hwnd` as the recipient of the session messages; unregistered in
        // WM_DESTROY.
        unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) }.is_ok()
    }

    fn notify(event: OsEvent) {
        if let Some(on_event) = ON_EVENT.get() {
            on_event(event);
        }
    }

    fn foreground_process_is(process: u32) -> bool {
        // SAFETY: these calls only read the foreground window and its owning process id.
        unsafe {
            let window = GetForegroundWindow();
            if window.0.is_null() {
                return false;
            }
            let mut foreground = 0;
            GetWindowThreadProcessId(window, Some(&mut foreground));
            foreground == process
        }
    }

    /// Exclude the window from capture and make it a (fully opaque) layered window.
    ///
    /// tao sets the affinity at creation; setting it again here is only a guard in case something
    /// reset it. The layered style is Electron's workaround for the black box a hidden-then-shown
    /// window leaves in captures (electron#29085, fixed by electron#31340; Tauri #14189 is the
    /// same bug). Re-setting the affinity alone most likely doesn't fix that: Windows still
    /// reports `WDA_EXCLUDEFROMCAPTURE` while it draws the black box, so the value doesn't change.
    /// Not yet tested on Windows here; it's on ADR-0002's checklist. tao rewrites the extended style on every show, hide and
    /// maximize, so `window_proc` keeps the bit in WM_STYLECHANGING.
    fn apply_capture_hiding(hwnd: HWND) {
        if crate::dev_allow_capture() {
            return;
        }
        CAPTURE_HIDING.store(true, Ordering::SeqCst);
        let style = ex_style(hwnd);
        // SAFETY: plain calls; user32 validates the handle. Electron sets the same bit on its
        // live windows. Alpha 255 with LWA_ALPHA draws the window as before: a layered window is
        // not drawn at all until its attributes are set.
        unsafe {
            if style & WS_EX_LAYERED.0 == 0 {
                let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, (style | WS_EX_LAYERED.0) as i32);
            }
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            let _ = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE);
        }
    }

    /// The main window's subclass procedure: sees every message before tao does, and passes on
    /// everything but our own retry timer.
    unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        match (msg, wparam.0 as u32) {
            (WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK) => notify(OsEvent::ScreenLocked),
            (
                WM_WTSSESSION_CHANGE,
                WTS_CONSOLE_DISCONNECT | WTS_REMOTE_DISCONNECT | WTS_SESSION_LOGOFF,
            ) => notify(OsEvent::UserSwitched),
            (WM_POWERBROADCAST, PBT_APMSUSPEND) => notify(OsEvent::Sleep),
            (WM_POWERBROADCAST, PBT_POWERSETTINGCHANGE) => {
                let setting = lparam.0 as *const POWERBROADCAST_SETTING;
                // SAFETY: for PBT_POWERSETTINGCHANGE, lParam points to a POWERBROADCAST_SETTING
                // valid for this call; `Data[0]` is inside the struct. For the display state it
                // is the low byte of a DWORD: 0 off, 1 on, 2 dimmed.
                let lock = !setting.is_null()
                    && unsafe {
                        let setting = &*setting;
                        (setting.PowerSetting == GUID_CONSOLE_DISPLAY_STATE
                            && setting.DataLength >= 1
                            && setting.Data[0] == 0)
                            || (setting.PowerSetting == GUID_LIDSWITCH_STATE_CHANGE
                                && setting.DataLength >= 4
                                && setting.Data[0] == 0)
                    };
                if lock {
                    notify(OsEvent::ScreenLocked);
                }
            }
            (WM_TIMER, SESSION_RETRY_TIMER) => {
                if register_session_notification(hwnd) {
                    LOCK_REGISTRATIONS
                        .fetch_or(super::LockRegistration::Session as u8, Ordering::SeqCst);
                    // SAFETY: our own timer on this window's thread.
                    let _ = unsafe { KillTimer(Some(hwnd), SESSION_RETRY_TIMER as usize) };
                }
                return LRESULT(0);
            }
            // wParam is GWL_EXSTYLE (-20) as a WPARAM; truncated to u32 like the arms above.
            (WM_STYLECHANGING, which)
                if which == GWL_EXSTYLE.0 as u32 && CAPTURE_HIDING.load(Ordering::SeqCst) =>
            {
                let style = lparam.0 as *mut STYLESTRUCT;
                // SAFETY: for WM_STYLECHANGING, lParam points to a STYLESTRUCT valid for this call
                // whose `styleNew` the window may change before it is applied.
                if !style.is_null() {
                    unsafe { (*style).styleNew |= WS_EX_LAYERED.0 };
                }
            }
            (WM_WINDOWPOSCHANGED, _) => {
                let pos = lparam.0 as *const WINDOWPOS;
                // SAFETY: for WM_WINDOWPOSCHANGED, lParam points to a WINDOWPOS valid for this call.
                if !pos.is_null() && unsafe { (*pos).flags.contains(SWP_SHOWWINDOW) } {
                    apply_capture_hiding(hwnd);
                }
            }
            (WM_SIZE, SIZE_MINIMIZED) => MINIMIZED.store(true, Ordering::SeqCst),
            (WM_SIZE, SIZE_RESTORED | SIZE_MAXIMIZED)
                if MINIMIZED.swap(false, Ordering::SeqCst) =>
            {
                apply_capture_hiding(hwnd);
            }
            (WM_DESTROY, _) => {
                FOREGROUND_RUNNING.store(false, Ordering::SeqCst);
                // SAFETY: our own timer on this still-live window's thread.
                let _ = unsafe { KillTimer(Some(hwnd), SESSION_RETRY_TIMER as usize) };
                super::unregister_lock_notifications(&LOCK_REGISTRATIONS, |kind| {
                    use super::LockRegistration::*;
                    // SAFETY: only successful registrations are undone, once, while hwnd exists.
                    unsafe {
                        match kind {
                            Session => {
                                let _ = WTSUnRegisterSessionNotification(hwnd);
                            }
                            DisplayOff | LidClose => {
                                let slot = if kind == DisplayOff {
                                    &POWER_NOTIFY
                                } else {
                                    &LID_NOTIFY
                                };
                                let handle = slot.swap(0, Ordering::SeqCst);
                                if handle != 0 {
                                    let _ =
                                        UnregisterPowerSettingNotification(HPOWERNOTIFY(handle));
                                }
                            }
                            WindowHook => {} // Removed in WM_NCDESTROY.
                        }
                    }
                });
            }
            (WM_NCDESTROY, _) => {
                // SAFETY: removes our own subclass on the window's thread, as documented.
                let _ = unsafe { RemoveWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID) };
            }
            _ => {}
        }
        // SAFETY: passes the message on unchanged to the next handler, as subclass procs must.
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
    }

    /// The home folder (Windows has no removable-volume lookup here yet).
    pub fn default_backup_folder(home: &Path) -> PathBuf {
        home.to_path_buf()
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod other {
    use std::path::{Path, PathBuf};

    use session::OsEvent;
    use tauri::{AppHandle, Runtime, WebviewWindow};

    use crate::core::{ApiError, Clipboard};

    pub fn capture_hiding_active<R: Runtime>(_window: &WebviewWindow<R>) -> Option<bool> {
        None
    }

    pub fn running_apps() -> Option<Vec<guard::RunningApp>> {
        Some(Vec::new())
    }

    pub fn clear_leftover_copy() {}

    pub fn real_home_dir() -> Option<PathBuf> {
        None
    }

    pub fn continuous_ms() -> Option<u64> {
        None
    }

    pub fn ms_since_last_input() -> Option<u64> {
        None
    }

    pub struct SystemClipboard;

    impl Clipboard for SystemClipboard {
        fn write(&mut self, _text: &str) -> Result<u64, ApiError> {
            Err(ApiError {
                code: "unsupported",
                message: "Copy isn't available on this computer yet.".to_owned(),
            })
        }

        fn clear_if(&mut self, _count: u64) -> bool {
            true
        }
    }

    pub fn observe_lock_events(
        _app: &AppHandle,
        _on_event: impl Fn(OsEvent) + Clone + Send + Sync + 'static,
    ) {
    }

    pub fn default_backup_folder(home: &Path) -> PathBuf {
        home.to_path_buf()
    }
}
