//! OS adapters: thin wrappers around platform APIs, one set per OS (ADR-0002, PRD module 6).
//!
//! * [`capture_hiding_active`]: the Capture Hiding self-check (main thread only).
//! * [`running_apps`]: what the Sharing Guard classifies.
//! * [`SystemClipboard`]: the clipboard, with our copies marked private (kept out of OS history
//!   and cloud sync; clipboard managers skip them only by convention). It never reads the
//!   clipboard's contents.
//! * [`observe_lock_events`]: OS notifications that must lock the app at once, and app switches.
//! * [`default_backup_folder`]: where the Backup save panel starts.
//!
//! macOS and Windows are implemented here; other platforms get harmless stand-ins (no self-check,
//! no running apps, no copy).
//!
//! This module and `hardening.rs` are the only places allowed to use `unsafe`; every block says
//! why it is sound.

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

    /// Bundle ids of the running applications.
    pub fn running_apps() -> Vec<guard::RunningApp> {
        NSWorkspace::sharedWorkspace()
            .runningApplications()
            .iter()
            .map(|app| guard::RunningApp {
                bundle_id: app.bundleIdentifier().map(|id| id.to_string()),
                exe_name: None,
            })
            .collect()
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
            let written = pasteboard.setString_forType(&NSString::from_str(text), string_type)
                && pasteboard
                    .setData_forType(Some(&marker), ns_string!("org.nspasteboard.ConcealedType"))
                && pasteboard
                    .setData_forType(Some(&marker), ns_string!("org.nspasteboard.TransientType"));
            if !written {
                pasteboard.clearContents();
                return Err(ApiError {
                    code: "io",
                    message: "The clipboard didn't accept the copy. Try again.".to_owned(),
                });
            }
            Ok(change_count(&pasteboard))
        }

        fn clear_if(&mut self, count: u64) {
            let pasteboard = NSPasteboard::generalPasteboard();
            if change_count(&pasteboard) == count {
                pasteboard.clearContents();
            }
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
}

#[cfg(windows)]
mod win {
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
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
    use windows::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
    use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
    use windows::Win32::UI::WindowsAndMessaging::{
        DEVICE_NOTIFY_WINDOW_HANDLE, GWL_EXSTYLE, GetWindowDisplayAffinity, GetWindowLongW,
        KillTimer, LWA_ALPHA, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE, SIZE_MAXIMIZED,
        SIZE_MINIMIZED, SIZE_RESTORED, STYLESTRUCT, SWP_SHOWWINDOW, SetLayeredWindowAttributes,
        SetTimer, SetWindowDisplayAffinity, SetWindowLongW, WDA_EXCLUDEFROMCAPTURE, WINDOWPOS,
        WM_DESTROY, WM_NCDESTROY, WM_POWERBROADCAST, WM_SIZE, WM_STYLECHANGING, WM_TIMER,
        WM_WINDOWPOSCHANGED, WM_WTSSESSION_CHANGE, WS_EX_LAYERED, WTS_CONSOLE_DISCONNECT,
        WTS_REMOTE_DISCONNECT, WTS_SESSION_LOCK, WTS_SESSION_LOGOFF,
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

    /// Executable file names of the running processes (e.g. "Zoom.exe").
    pub fn running_apps() -> Vec<guard::RunningApp> {
        let mut apps = Vec::new();
        // SAFETY: plain call with no pointers; the snapshot handle is closed below.
        let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return apps;
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
        apps
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

        fn clear_if(&mut self, count: u64) {
            // No owner window: this may run at exit, after the main window is gone.
            let Some(_open) = OpenedClipboard::open(None) else {
                return;
            };
            // Compared while we hold the clipboard, so nobody can write in between.
            // SAFETY: plain calls; the clipboard is open (`_open`).
            unsafe {
                if u64::from(GetClipboardSequenceNumber()) == count {
                    let _ = EmptyClipboard();
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
        // SAFETY: the clipboard didn't take `memory`, so it is still ours to free, once.
        let _ = unsafe { GlobalFree(Some(memory)) };
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
    /// Set while the main window is minimized, so restoring it re-applies Capture Hiding once.
    static MINIMIZED: AtomicBool = AtomicBool::new(false);
    /// Set once Capture Hiding is applied, so `WS_EX_LAYERED` survives tao's style rewrites.
    static CAPTURE_HIDING: AtomicBool = AtomicBool::new(false);
    /// The display-state registration's `HPOWERNOTIFY` (0 if none), undone in WM_DESTROY.
    static POWER_NOTIFY: AtomicIsize = AtomicIsize::new(0);
    const SUBCLASS_ID: usize = 1;
    /// Our WM_TIMER id for retrying the session registration; distinct from tao's timer ids.
    const SESSION_RETRY_TIMER: u32 = 0x454E_4F54;
    const SESSION_RETRY_MS: u32 = 2000;

    /// Call `on_event` for screen lock, display off (lid close), sleep, user switch, disconnect
    /// and log off, from the main window's messages; app switches arrive as window focus events.
    /// Also applies Capture Hiding (see [`apply_capture_hiding`]) and makes the main window the
    /// owner of our clipboard writes. Must be called on the main thread, which owns the window;
    /// lasts until it closes. A registration that fails is logged to stderr; the session one is
    /// retried every 2 s, since it fails while Terminal Services is still starting (an app started
    /// at log on).
    pub fn observe_lock_events(
        app: &AppHandle,
        on_event: impl Fn(OsEvent) + Clone + Send + Sync + 'static,
    ) {
        let Some(hwnd) = app.get_webview_window("main").as_ref().and_then(hwnd) else {
            return;
        };
        if ON_EVENT.get().is_some() {
            return;
        }
        CLIPBOARD_OWNER.store(hwnd.0 as isize, Ordering::SeqCst);
        apply_capture_hiding(hwnd);
        // SAFETY: we're on the thread that owns `hwnd` (SetWindowSubclass refuses others);
        // `window_proc` has the SUBCLASSPROC signature and uses no reference data.
        if !unsafe { SetWindowSubclass(hwnd, Some(window_proc), SUBCLASS_ID, 0) }.as_bool() {
            eprintln!(
                "encrypted-note: can't subclass the main window; screen lock, sleep and log off \
                 won't lock the app"
            );
            return;
        }
        let _ = ON_EVENT.set(Box::new(on_event));
        if !register_session_notification(hwnd) {
            eprintln!(
                "encrypted-note: session notifications unavailable (screen lock, user switch, \
                 log off); retrying every {SESSION_RETRY_MS} ms"
            );
            // SAFETY: plain call on the window's own thread; the timer dies with the window.
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
        // SAFETY: only names `hwnd` as the recipient of the display-state messages; the handle is
        // unregistered in WM_DESTROY.
        match unsafe {
            RegisterPowerSettingNotification(
                HANDLE(hwnd.0),
                &GUID_CONSOLE_DISPLAY_STATE,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        } {
            Ok(handle) => POWER_NOTIFY.store(handle.0, Ordering::SeqCst),
            Err(error) => eprintln!(
                "encrypted-note: display-off notifications unavailable ({error}); closing the lid \
                 locks the app only if the machine sleeps"
            ),
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
                let off = !setting.is_null()
                    && unsafe {
                        (*setting).PowerSetting == GUID_CONSOLE_DISPLAY_STATE
                            && (*setting).DataLength >= 1
                            && (*setting).Data[0] == 0
                    };
                if off {
                    notify(OsEvent::ScreenLocked);
                }
            }
            (WM_TIMER, SESSION_RETRY_TIMER) => {
                if register_session_notification(hwnd) {
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
                let power = POWER_NOTIFY.swap(0, Ordering::SeqCst);
                // SAFETY: the window still exists during WM_DESTROY, as unregistering requires;
                // `power` is the handle RegisterPowerSettingNotification returned, freed once.
                unsafe {
                    let _ = KillTimer(Some(hwnd), SESSION_RETRY_TIMER as usize);
                    let _ = WTSUnRegisterSessionNotification(hwnd);
                    if power != 0 {
                        let _ = UnregisterPowerSettingNotification(HPOWERNOTIFY(power));
                    }
                }
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

    pub fn running_apps() -> Vec<guard::RunningApp> {
        Vec::new()
    }

    pub struct SystemClipboard;

    impl Clipboard for SystemClipboard {
        fn write(&mut self, _text: &str) -> Result<u64, ApiError> {
            Err(ApiError {
                code: "unsupported",
                message: "Copy isn't available on this computer yet.".to_owned(),
            })
        }

        fn clear_if(&mut self, _count: u64) {}
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
