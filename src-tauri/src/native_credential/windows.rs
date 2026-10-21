//! Win32 password EDIT controls in a child overlay of the existing protected main HWND.
//! No dialog/top-level window is created. The WebView's original children are hidden and
//! disabled until the panel is cleared and destroyed.
use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, IsWindowEnabled, ReleaseCapture, SetCapture, SetFocus,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};
use zeroize::Zeroizing;

use super::{
    Activity, Callback, PresentationState, PresentationUpdate, Prompt, PromptError, Reply, Request,
    RevealMode, readout_text,
};

const SUBCLASS: usize = 0x454e_4352;
const ACCEPT: usize = 100;
const CANCEL: usize = 101;
const SUGGEST: usize = 102;
const HOLD: usize = 103;
const EXTEND: usize = 104;
const READOUT: usize = 105;
const REVEAL_TIMER: usize = 0xec20;
// Standard EDIT messages, defined by WinUser.h. This is the plain EDIT, never RichEdit.
const EM_LIMITTEXT: u32 = 0x00c5;
const EM_EMPTYUNDOBUFFER: u32 = 0x00cd;

thread_local! { static ACTIVE: RefCell<Option<Panel>> = const { RefCell::new(None) }; }

struct Panel {
    id: u64,
    owner: HWND,
    overlay: HWND,
    original: Vec<OriginalChild>,
    original_focus: HWND,
    fields: Vec<HWND>,
    sensitive_labels: Vec<HWND>,
    feedback: HWND,
    key_loop: Vec<HWND>,
    prompt: Request,
    activity: Option<Activity>,
    presentation: PresentationState,
    suggestion: Option<Zeroizing<String>>,
    completion: Callback,
    readout: Option<HWND>,
    readout_viewport: Option<HWND>,
    readout_shown: bool,
    readout_width: i32,
    layout_center: i32,
    extensions: u8,
    expires: Option<Instant>,
}

#[derive(Clone, Copy)]
struct OriginalChild {
    window: HWND,
    enabled: bool,
}

pub(super) fn present(
    window: &tauri::WebviewWindow,
    id: u64,
    prompt: Request,
    on_activity: Activity,
    completion: Callback,
) -> Result<(), PromptError> {
    let owner = window
        .hwnd()
        .map(|handle| HWND(handle.0))
        .map_err(|_| PromptError::Unavailable)?;
    // SAFETY: Both APIs only read thread identifiers; user32 validates the Tauri HWND.
    if unsafe { GetWindowThreadProcessId(owner, None) != GetCurrentThreadId() } {
        return Err(PromptError::Unavailable);
    }
    // SAFETY: Read-only foreground check; do not activate a background credential prompt.
    if unsafe { GetForegroundWindow() } != owner {
        return Err(PromptError::Unavailable);
    }
    if ACTIVE.with(|active| active.borrow().is_some()) {
        return Err(PromptError::Busy);
    }
    let mut bounds = RECT::default();
    // SAFETY: bounds is a writable local, and owner is a live window on this thread.
    unsafe { GetClientRect(owner, &mut bounds) }.map_err(|_| PromptError::Unavailable)?;
    let original = visible_children(owner);
    // SAFETY: Read this UI thread's focused control; validate it again before restoration.
    let original_focus = unsafe { GetFocus() };
    let overlay = control(
        owner,
        w!("STATIC"),
        "",
        WINDOW_STYLE(0),
        0,
        0,
        bounds.right,
        bounds.bottom,
        0,
    )?;
    let result = build(
        owner,
        overlay,
        original,
        original_focus,
        id,
        prompt,
        on_activity,
        completion,
        bounds.right,
    );
    let panel = match result {
        Ok(panel) => panel,
        Err(error) => {
            // SAFETY: Destroying this child releases any partially constructed controls.
            destroy_overlay(overlay);
            return Err(error);
        }
    };
    // SAFETY: The callbacks have Win32's exact SUBCLASSPROC ABI. Unique ID avoids the existing
    // capture/lock adapter's subclass. Controls and owner all belong to the current UI thread.
    if !unsafe { SetWindowSubclass(owner, Some(owner_proc), SUBCLASS, 0) }.as_bool()
        || !unsafe { SetWindowSubclass(overlay, Some(overlay_proc), SUBCLASS, 0) }.as_bool()
    {
        let _ = unsafe { RemoveWindowSubclass(owner, Some(owner_proc), SUBCLASS) };
        destroy_overlay(overlay);
        return Err(PromptError::Unavailable);
    }
    for control in &panel.key_loop {
        if !unsafe { SetWindowSubclass(*control, Some(input_proc), SUBCLASS, 0) }.as_bool() {
            let _ = unsafe { RemoveWindowSubclass(owner, Some(owner_proc), SUBCLASS) };
            destroy_overlay(overlay);
            return Err(PromptError::Unavailable);
        }
    }
    let first = panel.key_loop.first().copied();
    // SAFETY: Every panel revalidates deadlines/Auto-lock while idle or tracking native
    // scrollbars. These timer messages never count as owner input.
    if unsafe { SetTimer(Some(overlay), REVEAL_TIMER, 200, None) } == 0 {
        let _ = unsafe { RemoveWindowSubclass(owner, Some(owner_proc), SUBCLASS) };
        destroy_overlay(overlay);
        return Err(PromptError::Unavailable);
    }
    ACTIVE.with(|active| *active.borrow_mut() = Some(panel));
    activity(false);
    if !ACTIVE.with(|active| active.borrow().as_ref().is_some_and(|panel| panel.id == id)) {
        return Err(PromptError::Cancelled);
    }
    // SAFETY: No credential-bearing controls are visible until construction and registration
    // succeeded. Original children are restored only after all native text has been cleared.
    unsafe {
        for child in &ACTIVE.with(|active| {
            active
                .borrow()
                .as_ref()
                .map(|panel| panel.original.clone())
                .unwrap_or_default()
        }) {
            let _ = EnableWindow(child.window, false);
            let _ = ShowWindow(child.window, SW_HIDE);
        }
        let _ = SetWindowPos(
            overlay,
            Some(HWND_TOP),
            0,
            0,
            bounds.right,
            bounds.bottom,
            SWP_SHOWWINDOW,
        );
        if let Some(first) = first {
            let _ = SetFocus(Some(first));
        }
    }
    Ok(())
}

// Single-use native builder keeps the owner/restoration snapshot and moved credential
// request explicit so construction failures can clear the correct child overlay.
#[allow(clippy::too_many_arguments)]
fn build(
    owner: HWND,
    overlay: HWND,
    original: Vec<OriginalChild>,
    original_focus: HWND,
    id: u64,
    prompt: Request,
    activity: Activity,
    completion: Callback,
    window_width: i32,
) -> Result<Panel, PromptError> {
    let width = (window_width - 64).clamp(240, 560);
    let x = (window_width - width) / 2;
    let mut y = 28;
    let (title, detail) = prompt.text();
    label(overlay, title, x, y, width, 30)?;
    y += 38;
    let detail_height = match &prompt {
        Request::Entry(Prompt::NewPassword { .. }) => 48,
        Request::Entry(Prompt::RecoveryKit { .. }) => 64,
        _ => 160,
    };
    label(overlay, detail, x, y, width, detail_height)?;
    y += detail_height + 8;
    let mut sensitive_labels = Vec::new();
    let mut readout = None;
    let mut readout_viewport = None;
    let mut readout_shown = false;
    let readout_width = width - 24;
    let mut key_loop = Vec::new();
    let mut expires = None;
    if let Request::Reveal { reveal, .. } = &prompt {
        let viewport = control(
            overlay,
            w!("STATIC"),
            "",
            WS_VISIBLE | WS_TABSTOP | WS_VSCROLL | WS_CLIPCHILDREN,
            x,
            y,
            width,
            112,
            READOUT,
        )?;
        // SAFETY: Our unique subclass only handles scrolling on this live child HWND.
        if !unsafe { SetWindowSubclass(viewport, Some(viewport_proc), SUBCLASS, 0) }.as_bool() {
            return Err(PromptError::Unavailable);
        }
        let value = label(viewport, "", 0, 0, readout_width, 112)?;
        readout_shown = reveal.mode == RevealMode::Timed;
        set_readout(
            value,
            viewport,
            readout_width,
            if readout_shown {
                &reveal.value
            } else {
                "Hold Space or the button to show. Scroll to read long values."
            },
        );
        sensitive_labels.push(value);
        readout = Some(value);
        readout_viewport = Some(viewport);
        key_loop.push(viewport);
        y += 124;
        if reveal.mode == RevealMode::Held {
            key_loop.push(button(
                overlay,
                "Hold to show (or hold Space)",
                x,
                y,
                250,
                HOLD,
            )?);
        } else {
            key_loop.push(button(overlay, "Extend by 20 seconds", x, y, 200, EXTEND)?);
            expires = Some(Instant::now() + Duration::from_secs(20));
        }
        y += 44;
    }
    if let Request::Entry(Prompt::RecoveryKit { recovery, .. }) = &prompt {
        label(
            overlay,
            "Write this Recovery Key on paper and store it somewhere private.",
            x,
            y,
            width,
            32,
        )?;
        y += 38;
        sensitive_labels.push(label(overlay, recovery, x, y, width, 32)?);
        y += 40;
    }
    let mut fields = Vec::new();
    for caption in prompt.labels() {
        // A preceding STATIC label is the native accessibility label for the standard EDIT.
        label(overlay, caption, x, y, width, 22)?;
        y += 24;
        let field = control(
            overlay,
            w!("EDIT"),
            "",
            WS_VISIBLE
                | WS_TABSTOP
                | WS_BORDER
                | WINDOW_STYLE((ES_PASSWORD | ES_AUTOHSCROLL) as u32),
            x,
            y,
            width,
            28,
            200 + fields.len(),
        )?;
        // SAFETY: This is a standard live EDIT; set an explicit input/undo bound.
        unsafe {
            SendMessageW(field, EM_LIMITTEXT, Some(WPARAM(4096)), None);
        }
        fields.push(field);
        y += 36;
    }
    key_loop.extend_from_slice(&fields);
    if matches!(&prompt, Request::Entry(Prompt::NewPassword { .. })) {
        key_loop.push(button(overlay, "Suggest passphrase", x, y, 170, SUGGEST)?);
        y += 34;
        sensitive_labels.push(label(overlay, "", x, y, width, 36)?);
        y += 40;
    }
    let feedback = label(
        overlay,
        "Credentials stay in native controls. Escape cancels.",
        x,
        y,
        width,
        36,
    )?;
    y += 42;
    key_loop.push(button(overlay, "Cancel", x, y, 100, CANCEL)?);
    let is_reveal = matches!(&prompt, Request::Reveal { .. });
    key_loop.push(button(
        overlay,
        if is_reveal { "Close" } else { "Continue" },
        x + width - 112,
        y,
        112,
        ACCEPT,
    )?);
    Ok(Panel {
        id,
        owner,
        overlay,
        original,
        original_focus,
        fields,
        sensitive_labels,
        feedback,
        key_loop,
        prompt,
        completion,
        activity: Some(activity),
        presentation: PresentationState::default(),
        suggestion: None,
        readout,
        readout_viewport,
        readout_shown,
        readout_width,
        layout_center: x,
        extensions: 0,
        expires,
    })
}

// This thin CreateWindowExW wrapper intentionally preserves Win32 style/position/ID
// arguments; caller labels and native control classes remain visible at each creation.
#[allow(clippy::too_many_arguments)]
fn control(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: WINDOW_STYLE,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: usize,
) -> Result<HWND, PromptError> {
    let text = wide(text);
    // SAFETY: Class names are standard Win32 constants. Text is NUL-terminated and lives
    // through this synchronous copy. The child HWND is owned by its parent on this UI thread.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            PCWSTR(text.as_ptr()),
            WS_CHILD | style,
            x,
            y,
            width,
            height,
            Some(parent),
            Some(HMENU(id as *mut _)),
            None,
            None,
        )
    }
    .map_err(|_| PromptError::Unavailable)
}

fn label(
    parent: HWND,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<HWND, PromptError> {
    control(
        parent,
        w!("STATIC"),
        text,
        WS_VISIBLE,
        x,
        y,
        width,
        height,
        0,
    )
}

fn button(
    parent: HWND,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    id: usize,
) -> Result<HWND, PromptError> {
    control(
        parent,
        w!("BUTTON"),
        text,
        WS_VISIBLE | WS_TABSTOP,
        x,
        y,
        width,
        30,
        id,
    )
}

fn wide(text: &str) -> Zeroizing<Vec<u16>> {
    Zeroizing::new(text.encode_utf16().chain(Some(0)).collect())
}

fn set_text(control: HWND, text: &str) {
    let text = wide(text);
    // SAFETY: text stays valid until this synchronous user32 copy completes.
    let _ = unsafe { SetWindowTextW(control, PCWSTR(text.as_ptr())) };
}

fn destroy_overlay(overlay: HWND) {
    // Clear every partially created child too: a failed construction must not leave a
    // Recovery Key or suggestion in a native label until user32 frees its backing storage.
    unsafe {
        let mut next = GetWindow(overlay, GW_CHILD).ok();
        while let Some(child) = next {
            clear_control_tree(child);
            next = GetWindow(child, GW_HWNDNEXT).ok();
        }
        let _ = DestroyWindow(overlay);
    }
}

unsafe fn clear_control_tree(control: HWND) {
    // SAFETY: Called only on descendants of our native overlay, all on this UI thread.
    unsafe {
        let mut next = GetWindow(control, GW_CHILD).ok();
        while let Some(child) = next {
            clear_control_tree(child);
            next = GetWindow(child, GW_HWNDNEXT).ok();
        }
        set_text(control, "");
        SendMessageW(control, EM_EMPTYUNDOBUFFER, None, None);
    }
}

fn visible_children(owner: HWND) -> Vec<OriginalChild> {
    let mut children = Vec::new();
    // SAFETY: user32 validates all handles. Only direct, visible children are saved/restored;
    // descendants remain controlled by their original parent, avoiding accidental disclosure.
    unsafe {
        let mut next = GetWindow(owner, GW_CHILD).ok();
        while let Some(child) = next {
            if IsWindowVisible(child).as_bool() {
                children.push(OriginalChild {
                    window: child,
                    enabled: IsWindowEnabled(child).as_bool(),
                });
            }
            next = GetWindow(child, GW_HWNDNEXT).ok();
        }
    }
    children
}

fn read_text(field: HWND) -> Result<Zeroizing<String>, &'static str> {
    let mut buffer = Zeroizing::new(vec![0u16; 4097]);
    // SAFETY: This EDIT belongs to this process/thread. The live 4097-element buffer includes
    // the terminator; GetWindowTextW receives its actual size and cannot write past it.
    let count = unsafe { GetWindowTextW(field, &mut buffer) };
    let count = usize::try_from(count).map_err(|_| "The native field could not be read.")?;
    String::from_utf16(&buffer[..count])
        .map(Zeroizing::new)
        .map_err(|_| "Use valid text in the credential field.")
}

fn submit() {
    if !activity(false) {
        return;
    }
    if ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(|panel| matches!(&panel.prompt, Request::Reveal { .. }))
    }) {
        finish(Err(PromptError::Cancelled));
        return;
    }
    let reply = ACTIVE.with(|active| {
        let active = active.borrow();
        let panel = active.as_ref()?;
        let values = panel
            .fields
            .iter()
            .map(|field| read_text(*field))
            .collect::<Result<Vec<_>, _>>();
        match values.and_then(|values| panel.prompt.reply(values)) {
            Ok(reply) => Some(reply),
            Err(message) => {
                set_text(panel.feedback, message);
                None
            }
        }
    });
    if let Some(reply) = reply {
        finish(Ok(reply));
    }
}

fn held(show: bool) {
    if show && !activity(false) {
        return;
    }
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let Some(panel) = active.as_mut() else {
            return;
        };
        if let (Request::Reveal { reveal, .. }, Some(readout), Some(viewport)) =
            (&panel.prompt, panel.readout, panel.readout_viewport)
            && reveal.mode == RevealMode::Held
            && panel.readout_shown != show
        {
            panel.readout_shown = show;
            set_readout(
                readout,
                viewport,
                panel.readout_width,
                if show {
                    &reveal.value
                } else {
                    "Hold Space or the button to show. Scroll to read long values."
                },
            );
        }
    });
}

fn set_readout(readout: HWND, viewport: HWND, width: i32, value: &str) {
    let (display, lines) = readout_text(value);
    let height = i32::try_from(lines)
        .unwrap_or(i32::MAX / 24)
        .saturating_mul(24)
        .saturating_add(12)
        .max(112);
    set_text(readout, &display);
    let info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: height - 1,
        nPage: 112,
        nPos: 0,
        nTrackPos: 0,
    };
    // SAFETY: The control is our nonselectable STATIC child of this native viewport. Reset
    // scroll metadata as well as text on held release, so hidden AX has no old value/extent.
    unsafe {
        SetScrollInfo(viewport, SB_VERT, &info, true);
        let _ = MoveWindow(readout, 0, 0, width, height, true);
    }
}

fn scroll_readout(command: Option<i32>, delta: i32) -> bool {
    if !activity(false) {
        return false;
    }
    ACTIVE.with(|active| {
        let active = active.borrow();
        let Some(panel) = active.as_ref() else {
            return false;
        };
        let (Some(viewport), Some(readout)) = (panel.readout_viewport, panel.readout) else {
            return false;
        };
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        // SAFETY: Local SCROLLINFO has its actual size. Only our live native child is moved;
        // its parent clips it, keeping every glyph inside the protected main window.
        unsafe {
            if GetScrollInfo(viewport, SB_VERT, &mut info).is_err() {
                return false;
            }
            let maximum = (info.nMax - info.nPage as i32 + 1).max(0);
            let next = match command {
                Some(0) => info.nPos.saturating_sub(24),
                Some(1) => info.nPos.saturating_add(24),
                Some(2) => info.nPos.saturating_sub(112),
                Some(3) => info.nPos.saturating_add(112),
                Some(4 | 5) => info.nTrackPos,
                Some(6) => 0,
                Some(7) => maximum,
                _ => info.nPos.saturating_add(delta),
            }
            .clamp(0, maximum);
            info.fMask = SIF_POS;
            info.nPos = next;
            SetScrollInfo(viewport, SB_VERT, &info, true);
            let _ = MoveWindow(
                readout,
                0,
                -next,
                panel.readout_width,
                (info.nMax + 1).max(112),
                true,
            );
        }
        true
    })
}

fn extend() {
    if !activity(false) {
        return;
    }
    let pending = ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let panel = active.as_mut()?;
        let Request::Reveal { reveal, extend } = &mut panel.prompt else {
            return None;
        };
        if reveal.mode != RevealMode::Timed || panel.extensions >= 3 {
            return None;
        }
        extend.take().map(|hook| (panel.id, hook))
    });
    if let Some((id, hook)) = pending {
        let allowed = hook();
        let present = ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            let Some(panel) = active.as_mut().filter(|panel| panel.id == id) else {
                return false;
            };
            if let Request::Reveal { extend, .. } = &mut panel.prompt {
                *extend = Some(hook);
            }
            if allowed {
                panel.extensions += 1;
                panel.expires = Some(Instant::now() + Duration::from_secs(20));
            }
            true
        });
        if present && !allowed {
            finish(Err(PromptError::Cancelled));
            return;
        }
    }
    tick();
}

fn tick() {
    if !activity(false) {
        return;
    }
    let expired = ACTIVE.with(|active| {
        let active = active.borrow();
        let Some(panel) = active.as_ref() else {
            return false;
        };
        let Some(expires) = panel.expires else {
            return false;
        };
        let remaining = expires.saturating_duration_since(Instant::now());
        set_text(
            panel.feedback,
            &format!(
                "Hides in {} seconds. {} extensions remaining.",
                remaining.as_secs() + 1,
                3 - panel.extensions
            ),
        );
        remaining.is_zero()
    });
    if expired {
        finish(Err(PromptError::Cancelled));
    }
}

fn forget_suggestion() {
    ACTIVE.with(|active| {
        // SetWindowTextW during suggestion installation sends EN_CHANGE synchronously.
        // Ignore that reentrant notification; owner edits arrive after the builder returns.
        if let Ok(mut active) = active.try_borrow_mut()
            && let Some(panel) = active.as_mut()
            && panel.suggestion.take().is_some()
            && let Some(label) = panel.sensitive_labels.last()
        {
            set_text(*label, "");
        }
    });
}

fn suggest() {
    if !activity(false) {
        return;
    }
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        let Some(panel) = active.as_mut() else {
            return;
        };
        if !matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. })) {
            return;
        }
        let suggestion = policy::suggest_passphrase();
        for field in panel.fields.iter().rev().take(2) {
            set_text(*field, &suggestion);
        }
        if let Some(label) = panel.sensitive_labels.last() {
            set_text(*label, &suggestion);
        }
        panel.suggestion = Some(suggestion);
        set_text(
            panel.feedback,
            "Suggested passphrase meets the strength rules. Write it somewhere private.",
        );
    });
}

fn finish(result: Result<Reply, PromptError>) {
    let panel = ACTIVE.with(|active| active.borrow_mut().take());
    let Some(panel) = panel else {
        return;
    };
    for field in &panel.fields {
        set_text(*field, "");
        // SAFETY: This live EDIT's undo buffer is cleared before destruction.
        unsafe {
            SendMessageW(*field, EM_EMPTYUNDOBUFFER, None, None);
        }
    }
    for label in &panel.sensitive_labels {
        set_text(*label, "");
    }
    // SAFETY: Every child belongs to this UI thread. A closed/replaced owner is handled by
    // IsWindow; user32 validates saved original handles. Credentials are cleared first.
    unsafe {
        let _ = KillTimer(Some(panel.overlay), REVEAL_TIMER);
        let _ = RemoveWindowSubclass(panel.owner, Some(owner_proc), SUBCLASS);
        destroy_overlay(panel.overlay);
        if IsWindow(Some(panel.owner)).as_bool() {
            for child in &panel.original {
                if IsWindow(Some(child.window)).as_bool() {
                    let _ = EnableWindow(child.window, child.enabled);
                    let _ = ShowWindow(child.window, SW_SHOW);
                }
            }
        }
        if GetForegroundWindow() == panel.owner
            && IsWindow(Some(panel.original_focus)).as_bool()
            && IsChild(panel.owner, panel.original_focus).as_bool()
        {
            let _ = SetFocus(Some(panel.original_focus));
        }
    }
    panel.completion.finish(result);
}

pub(super) fn cancel(id: u64) -> bool {
    let matches =
        ACTIVE.with(|active| active.borrow().as_ref().is_some_and(|panel| panel.id == id));
    if matches {
        finish(Err(PromptError::Cancelled));
    }
    matches
}

unsafe extern "system" fn overlay_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_TIMER && wparam.0 == REVEAL_TIMER {
        tick();
        return LRESULT(0);
    }
    if message == WM_COMMAND && (wparam.0 >> 16) == 0x0300 {
        forget_suggestion();
    }
    if message == WM_COMMAND && (wparam.0 >> 16) == 0 {
        if (wparam.0 & 0xffff) == CANCEL {
            finish(Err(PromptError::Cancelled));
            return LRESULT(0);
        }
        if !activity(true) {
            return LRESULT(0);
        }
        match wparam.0 & 0xffff {
            ACCEPT => submit(),
            CANCEL => finish(Err(PromptError::Cancelled)),
            SUGGEST => suggest(),
            EXTEND => extend(),
            _ => {}
        }
        return LRESULT(0);
    }
    // SAFETY: Forward the unchanged event through the OS-managed subclass chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn viewport_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_VSCROLL {
        scroll_readout(Some((wparam.0 & 0xffff) as i32), 0);
        return LRESULT(0);
    }
    // SAFETY: Preserve all unrelated STATIC/scrollbar messages through the OS subclass chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn owner_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_CLOSE
        || message == WM_NCDESTROY
        || (message == WM_ACTIVATE && (wparam.0 & 0xffff) == 0)
    {
        finish(Err(PromptError::Cancelled));
    }
    if message == WM_SIZE {
        let mut bounds = RECT::default();
        // SAFETY: The local rectangle is writable, and this callback belongs to our owner.
        if unsafe { GetClientRect(hwnd, &mut bounds) }.is_ok() {
            let resize = ACTIVE.with(|active| {
                let mut active = active.borrow_mut();
                let panel = active.as_mut()?;
                let center = (bounds.right - (panel.readout_width + 24)) / 2;
                let delta = center - panel.layout_center;
                panel.layout_center = center;
                Some((panel.overlay, delta))
            });
            if let Some((overlay, delta)) = resize {
                // SAFETY: The overlay and its direct children belong to this UI thread.
                // Move children after dropping ACTIVE's borrow: user32 may dispatch messages.
                unsafe {
                    let _ = SetWindowPos(
                        overlay,
                        Some(HWND_TOP),
                        0,
                        0,
                        bounds.right,
                        bounds.bottom,
                        SWP_NOACTIVATE,
                    );
                    if delta != 0 {
                        let mut outer = RECT::default();
                        if GetWindowRect(overlay, &mut outer).is_ok() {
                            let mut next = GetWindow(overlay, GW_CHILD).ok();
                            while let Some(child) = next {
                                let mut rect = RECT::default();
                                if GetWindowRect(child, &mut rect).is_ok() {
                                    let _ = MoveWindow(
                                        child,
                                        rect.left - outer.left + delta,
                                        rect.top - outer.top,
                                        rect.right - rect.left,
                                        rect.bottom - rect.top,
                                        true,
                                    );
                                }
                                next = GetWindow(child, GW_HWNDNEXT).ok();
                            }
                        }
                    }
                }
            }
        }
    }
    // SAFETY: Forward the unchanged event through the existing capture/lock adapter's chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn input_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_KEYDOWN && wparam.0 == 0x1b {
        finish(Err(PromptError::Cancelled));
        return LRESULT(0);
    }
    // Bit 30 identifies an already-held key. Repeated key-downs must not defeat Auto-lock.
    if (matches!(message, WM_LBUTTONDOWN | WM_NCLBUTTONDOWN)
        || (message == WM_KEYDOWN && (lparam.0 & (1 << 30)) == 0)
        || (message == WM_CHAR && (lparam.0 & (1 << 30)) == 0))
        && !activity(true)
    {
        return LRESULT(0);
    }
    if message == WM_MOUSEWHEEL {
        if !activity(true) {
            return LRESULT(0);
        }
        let delta = ((wparam.0 >> 16) as u16 as i16) as i32;
        if scroll_readout(None, -delta / 2) {
            return LRESULT(0);
        }
    }
    if message == WM_KEYDOWN {
        let command = match wparam.0 {
            0x26 => Some(0),
            0x28 => Some(1),
            0x21 => Some(2),
            0x22 => Some(3),
            0x24 => Some(6),
            0x23 => Some(7),
            _ => None,
        };
        if let Some(command) = command
            && scroll_readout(Some(command), 0)
        {
            return LRESULT(0);
        }
    }
    // SAFETY: GetDlgCtrlID reads the native child ID assigned during construction.
    let control_id = unsafe { GetDlgCtrlID(hwnd) };
    let is_hold = control_id == HOLD as i32 || (control_id == READOUT as i32 && ACTIVE.with(|active| active.borrow().as_ref().is_some_and(|panel| matches!(&panel.prompt, Request::Reveal { reveal, .. } if reveal.mode == RevealMode::Held))));
    if is_hold {
        if message == WM_LBUTTONDOWN {
            // SAFETY: Capture keeps mouse-up delivery on this control if the pointer leaves it.
            unsafe {
                let _ = SetFocus(Some(hwnd));
                SetCapture(hwnd);
            }
            held(true);
            return LRESULT(0);
        }
        if matches!(
            message,
            WM_LBUTTONUP | WM_CAPTURECHANGED | WM_CANCELMODE | WM_KILLFOCUS
        ) {
            held(false);
            if message == WM_LBUTTONUP {
                let _ = unsafe { ReleaseCapture() };
            }
            return LRESULT(0);
        }
        if message == WM_KEYDOWN && wparam.0 == 0x20 {
            held(true);
            return LRESULT(0);
        }
        if message == WM_KEYUP && wparam.0 == 0x20 {
            held(false);
            return LRESULT(0);
        }
        if message == WM_CHAR && wparam.0 == 0x20 {
            return LRESULT(0);
        }
    }
    // Standard password EDIT masks display but is not a complete clipboard policy. Block
    // explicit messages and keyboard shortcuts; plain EDIT has no OLE drag source enabled.
    if matches!(message, WM_COPY | WM_CUT | WM_CONTEXTMENU) {
        return LRESULT(0);
    }
    if message == WM_CHAR && matches!(wparam.0, 3 | 24 | 9 | 13 | 27) {
        return LRESULT(0);
    }
    if message == WM_GETDLGCODE {
        return LRESULT(DLGC_WANTALLKEYS as isize);
    }
    if message == WM_KEYDOWN {
        // SAFETY: GetKeyState only reads keyboard state; it observes no credential text.
        let control = unsafe { GetKeyState(0x11) } < 0;
        let shift = unsafe { GetKeyState(0x10) } < 0;
        if (control && matches!(wparam.0, 0x43 | 0x58 | 0x2d)) || (shift && wparam.0 == 0x2e) {
            return LRESULT(0);
        }
        match wparam.0 {
            0x1b => {
                finish(Err(PromptError::Cancelled));
                return LRESULT(0);
            }
            0x0d => {
                submit();
                return LRESULT(0);
            }
            0x09 => {
                let next = ACTIVE.with(|active| {
                    let active = active.borrow();
                    let panel = active.as_ref()?;
                    let at = panel.key_loop.iter().position(|control| *control == hwnd)?;
                    let count = panel.key_loop.len();
                    panel
                        .key_loop
                        .get((at + if shift { count - 1 } else { 1 }) % count)
                        .copied()
                });
                // SAFETY: next is a live native panel control on this thread.
                if let Some(next) = next {
                    let _ = unsafe { SetFocus(Some(next)) };
                }
                return LRESULT(0);
            }
            _ => {}
        }
    }
    // SAFETY: Forward input unchanged through the standard EDIT/BUTTON subclass chain.
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn activity(genuine: bool) -> bool {
    // Remove the hook while invoking it, so a rejected lease can dismiss/reenter without
    // holding the native panel RefCell borrow. Reinsert only into the exact same request.
    let pending = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        panel.activity.take().map(|hook| (panel.id, hook))
    });
    let Some((id, hook)) = pending else {
        return false;
    };
    let authorization = hook(genuine);
    let update = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut().filter(|panel| panel.id == id)?;
        panel.activity = Some(hook);
        let update = panel.presentation.authorize(authorization);
        match update {
            PresentationUpdate::Clear => {
                for label in &panel.sensitive_labels {
                    set_text(*label, "");
                }
                if let (Some(readout), Some(viewport)) = (panel.readout, panel.readout_viewport) {
                    set_readout(readout, viewport, panel.readout_width, "");
                }
                panel.readout_shown = false;
                set_text(panel.feedback, "Please wait…");
            }
            PresentationUpdate::Restore => {
                if let Request::Entry(Prompt::RecoveryKit { recovery, .. }) = &panel.prompt
                    && let Some(label) = panel.sensitive_labels.first()
                {
                    set_text(*label, recovery);
                }
                if let (Request::Reveal { reveal, .. }, Some(readout), Some(viewport)) =
                    (&panel.prompt, panel.readout, panel.readout_viewport)
                {
                    panel.readout_shown = reveal.mode == RevealMode::Timed
                        && panel
                            .expires
                            .is_some_and(|expires| Instant::now() < expires);
                    set_readout(
                        readout,
                        viewport,
                        panel.readout_width,
                        if panel.readout_shown {
                            &reveal.value
                        } else {
                            "Hold Space or the button to show. Scroll to read long values."
                        },
                    );
                }
                if let Some(suggestion) = &panel.suggestion
                    && let Some(label) = panel.sensitive_labels.last()
                {
                    set_text(*label, suggestion);
                }
                // Restore only a generated candidate, never a manually typed password.
                set_text(
                    panel.feedback,
                    "Credentials stay in native controls. Escape cancels.",
                );
            }
            _ => (),
        }
        Some(update)
    });
    if update == Some(PresentationUpdate::Close) {
        finish(Err(PromptError::Cancelled));
    }
    matches!(
        update,
        Some(PresentationUpdate::Keep | PresentationUpdate::Restore)
    )
}
