//! Win32 password EDIT controls in a child overlay of the existing protected main HWND.
//! No dialog/top-level window is created. The WebView's original children are hidden and
//! disabled until the panel is cleared and destroyed.
use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::System::SystemServices::SS_NOPREFIX;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::Input::Ime::{HIMC, ImmAssociateContext};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, IsWindowEnabled, ReleaseCapture, SetCapture, SetFocus,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{PCWSTR, w};
use zeroize::Zeroizing;

use super::{
    Activity, ArmDecision, Authorization, CAPS_LOCK_TEXT, Callback, CancelArm, KIT_ARMED,
    KIT_PAPER_NOTE, LiveCheck, MayShow, PresentationState, PresentationUpdate, Prompt, PromptError,
    RESET_BUTTON, Refusal, RefusalFocus, Reply, Request, RevealMode, SUGGEST_BUTTON,
    SUGGESTED_TEXT, SUGGESTION_ASK, SUGGESTION_REMOVED, Suggestion, TimeLeft, TimeLimit, TimeProbe,
    kept_refusal_box, problem_text, readout_text, recovery_progress, timed_readout_text,
};

const SUBCLASS: usize = 0x454e_4352;
const ACCEPT: usize = 100;
const CANCEL: usize = 101;
const SUGGEST: usize = 102;
const HOLD: usize = 103;
const EXTEND: usize = 104;
const READOUT: usize = 105;
/// Boxes take control ids from here up, in the order of `Panel::fields`.
const FIRST_FIELD: usize = 200;
const REVEAL_TIMER: usize = 0xec20;
// Standard EDIT messages, defined by WinUser.h. This is the plain EDIT, never RichEdit.
const EM_LIMITTEXT: u32 = 0x00c5;
const EM_SETPASSWORDCHAR: u32 = 0x00cc;
const EM_EMPTYUNDOBUFFER: u32 = 0x00cd;
const EM_SETCUEBANNER: u32 = 0x1501;
const BULLET: usize = 0x25cf;
// Three lines and two, by the same estimate as `detail_height`.
const KIT_NOTE_HEIGHT: i32 = 60;
const FEEDBACK_HEIGHT: i32 = 40;

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
    time_row: HWND,
    key_loop: Vec<HWND>,
    prompt: Request,
    activity: Option<Activity>,
    time_left: TimeProbe,
    presentation: PresentationState,
    suggestion: Option<Suggestion>,
    /// Set while this code writes to the boxes: those EN_CHANGE messages aren't the owner's edits.
    installing: bool,
    /// The last refusal, shown again after a Retry. Holds counts and group numbers only.
    last_error: Option<String>,
    /// The row holds a refusal or notice to read, so the live lines leave it alone until an edit.
    error_shown: bool,
    cancel_arm: CancelArm,
    live: LiveCheck,
    /// The newest live line, so a Retry or a cleared refusal doesn't lose it. Holds no typed text.
    live_line: Option<String>,
    completion: Callback,
    readout: Option<HWND>,
    readout_viewport: Option<HWND>,
    readout_shown: bool,
    readout_width: i32,
    layout_center: i32,
    layout_top: i32,
    panel_height: i32,
    extensions: u8,
    expires: Option<Instant>,
}

impl Panel {
    /// The feedback row when no refusal or question is waiting to be read.
    fn normal_feedback(&self) -> String {
        if self.suggestion.is_some() {
            return SUGGESTED_TEXT.to_owned();
        }
        if let Some(line) = &self.live_line {
            return line.clone();
        }
        self.prompt.feedback_text()
    }

    /// What the row goes back to after a Retry, or after a first Cancel on the Kit expires.
    fn restored_feedback(&mut self) -> String {
        self.error_shown = self.last_error.is_some();
        match &self.last_error {
            Some(message) => problem_text(message),
            None => self.normal_feedback(),
        }
    }
}

#[derive(Clone, Copy)]
struct OriginalChild {
    window: HWND,
    enabled: bool,
}

// may_show is for the Show what I type tickbox, which is Mac only.
pub(super) fn present(
    window: &tauri::WebviewWindow,
    id: u64,
    prompt: Request,
    on_activity: Activity,
    time_left: TimeProbe,
    _may_show: MayShow,
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
        time_left,
        completion,
        bounds.right,
        bounds.bottom,
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
    show_recovery_progress(id);
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
    time_left: TimeProbe,
    completion: Callback,
    window_width: i32,
    window_height: i32,
) -> Result<Panel, PromptError> {
    let width = (window_width - 64).clamp(240, 560);
    let x = (window_width - width) / 2;
    let (title, detail) = prompt.text();
    let detail_height = detail_height(detail, width);
    let time_height = time_row_height(&prompt, width);
    let height = panel_height(&prompt, detail_height, time_height);
    let top = panel_top(window_height, height);
    let mut y = top;
    label(overlay, title, x, y, width, 30)?;
    y += 38;
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
            key_loop.push(button(overlay, RESET_BUTTON, x, y, 200, EXTEND)?);
            expires = Some(Instant::now() + Duration::from_millis(session::ACCESSIBLE_REVEAL_MS));
        }
        y += 44;
    }
    if let Request::Entry(Prompt::RecoveryKit { recovery, .. }) = &prompt {
        label(overlay, KIT_PAPER_NOTE, x, y, width, KIT_NOTE_HEIGHT)?;
        y += KIT_NOTE_HEIGHT + 6;
        sensitive_labels.push(label(overlay, recovery, x, y, width, 32)?);
        y += 40;
    }
    let mut fields = Vec::new();
    for caption in prompt.labels() {
        // A preceding STATIC label is the native accessibility label for the standard EDIT.
        label(overlay, caption, x, y, width, 22)?;
        y += 24;
        let masking = if prompt.shows_typing() {
            0
        } else {
            ES_PASSWORD
        };
        let field = control(
            overlay,
            w!("EDIT"),
            "",
            WS_VISIBLE | WS_TABSTOP | WS_BORDER | WINDOW_STYLE((masking | ES_AUTOHSCROLL) as u32),
            x,
            y,
            width,
            28,
            FIRST_FIELD + fields.len(),
        )?;
        if prompt.shows_typing() {
            // A visible box would otherwise hand what is typed to an input method's own windows
            // and any cloud input service. A null context turns the input method off for it.
            // SAFETY: field is a live EDIT on this thread; no context is created or destroyed.
            unsafe {
                ImmAssociateContext(field, HIMC::default());
            }
            let cue = wide("XXXX-XXXX");
            // SAFETY: The EDIT copies this NUL-terminated text before SendMessageW returns.
            // wParam 1 keeps the cue while the box has focus, which it has from the start.
            unsafe {
                SendMessageW(
                    field,
                    EM_SETCUEBANNER,
                    Some(WPARAM(1)),
                    Some(LPARAM(cue.as_ptr() as isize)),
                );
            }
        }
        // SAFETY: This is a standard live EDIT; set an explicit input/undo bound.
        unsafe {
            SendMessageW(field, EM_LIMITTEXT, Some(WPARAM(4096)), None);
        }
        fields.push(field);
        y += 36;
    }
    key_loop.extend_from_slice(&fields);
    if matches!(&prompt, Request::Entry(Prompt::NewPassword { .. })) {
        key_loop.push(button(overlay, SUGGEST_BUTTON, x, y, 170, SUGGEST)?);
        y += 34;
        sensitive_labels.push(label(overlay, "", x, y, width, 48)?);
        y += 52;
    }
    let feedback = label(
        overlay,
        &prompt.feedback_text(),
        x,
        y,
        width,
        FEEDBACK_HEIGHT,
    )?;
    y += FEEDBACK_HEIGHT + 6;
    // Not in sensitive_labels: this row only ever says how much time is left.
    let time_row = label(overlay, "", x, y, width, time_height)?;
    y += time_height + 4;
    debug_assert_eq!(y + 30 - top, height);
    // A readout has one button, which keeps Escape. Nothing here hints that a copy can be undone.
    if matches!(&prompt, Request::Reveal { .. }) {
        key_loop.push(button(overlay, "Close", x + width - 112, y, 112, CANCEL)?);
    } else {
        key_loop.push(button(overlay, "Cancel", x, y, 100, CANCEL)?);
        key_loop.push(button(
            overlay,
            "Continue",
            x + width - 112,
            y,
            112,
            ACCEPT,
        )?);
    }
    Ok(Panel {
        id,
        owner,
        overlay,
        original,
        original_focus,
        fields,
        sensitive_labels,
        feedback,
        time_row,
        key_loop,
        prompt,
        completion,
        activity: Some(activity),
        time_left,
        presentation: PresentationState::default(),
        suggestion: None,
        installing: false,
        last_error: None,
        error_shown: false,
        cancel_arm: CancelArm::default(),
        live: LiveCheck::default(),
        live_line: None,
        readout,
        readout_viewport,
        readout_shown,
        readout_width,
        layout_center: x,
        layout_top: top,
        panel_height: height,
        extensions: 0,
        expires,
    })
}

/// An estimate from the text's length, because nothing here can measure or see the Win32 font:
/// about 8 pixels a character and 20 a line. Line breaks and wide characters are counted.
fn detail_height(detail: &str, width: i32) -> i32 {
    let per_line = usize::try_from(width / 8).unwrap_or(1).max(1);
    let lines: usize = detail
        .split('\n')
        .map(|line| {
            let columns: usize = line.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
            columns.div_ceil(per_line).max(1)
        })
        .sum();
    i32::try_from(lines)
        .unwrap_or(i32::MAX / 20)
        .saturating_mul(20)
        .min(216)
}

/// Two lines, or more if this panel's longest warning needs them, so it is never cut off.
fn time_row_height(prompt: &Request, width: i32) -> i32 {
    let longest = prompt
        .time_row_text(&TimeLeft::Left {
            seconds: 60,
            limit: TimeLimit::AutoLock,
        })
        .unwrap_or_default();
    detail_height(&longest, width).max(40)
}

/// Every row `build` places, top to bottom. Its debug assertion keeps the two in step.
fn panel_height(prompt: &Request, detail_height: i32, time_height: i32) -> i32 {
    let extra = match prompt {
        Request::Reveal { .. } => 124 + 44,
        Request::Entry(Prompt::RecoveryKit { .. }) => KIT_NOTE_HEIGHT + 6 + 40,
        Request::Entry(Prompt::NewPassword { .. }) => 34 + 52,
        _ => 0,
    };
    let boxes = i32::try_from(prompt.labels().len()).unwrap_or(0);
    38 + detail_height + 8 + extra + boxes * 60 + FEEDBACK_HEIGHT + 6 + time_height + 4 + 30
}

/// Centered, but a panel taller than the window keeps its title in view.
fn panel_top(window_height: i32, panel_height: i32) -> i32 {
    ((window_height - panel_height) / 2).max(28)
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
    // SS_NOPREFIX: a STATIC would otherwise hide each & in a value and underline the next letter.
    control(
        parent,
        w!("STATIC"),
        text,
        WS_VISIBLE | WINDOW_STYLE(SS_NOPREFIX.0),
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

/// For rows the timer rewrites, which hold app wording only: the same text again would flicker.
fn set_text_if_changed(control: HWND, text: &str) {
    let mut shown = [0u16; 512];
    // SAFETY: This STATIC belongs to this thread, and GetWindowTextW gets the buffer's real size.
    let count = unsafe { GetWindowTextW(control, &mut shown) };
    let same = usize::try_from(count)
        .ok()
        .and_then(|count| shown.get(..count))
        .is_some_and(|shown| shown.iter().copied().eq(text.encode_utf16()));
    if !same {
        set_text(control, text);
    }
}

fn clear_box(field: HWND) {
    set_text(field, "");
    // SAFETY: This live EDIT's undo buffer is cleared along with its text.
    unsafe {
        SendMessageW(field, EM_EMPTYUNDOBUFFER, None, None);
    }
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

fn read_text(field: HWND) -> Result<Zeroizing<String>, Refusal> {
    let mut buffer = Zeroizing::new(vec![0u16; 4097]);
    // SAFETY: This EDIT belongs to this process/thread. The live 4097-element buffer includes
    // the terminator; GetWindowTextW receives its actual size and cannot write past it.
    let count = unsafe { GetWindowTextW(field, &mut buffer) };
    let count = usize::try_from(count)
        .map_err(|_| Refusal::keep("That box couldn't be read. Clear it and type it again."))?;
    String::from_utf16(&buffer[..count])
        .map(Zeroizing::new)
        .map_err(|_| {
            Refusal::keep(
                "That text has characters the app can't read. Clear the box and type it again.",
            )
        })
}

fn submit() {
    if !activity(false) {
        return;
    }
    let target = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        panel.cancel_arm.disarm();
        Some((
            panel.id,
            panel.fields.clone(),
            matches!(&panel.prompt, Request::Reveal { .. }),
        ))
    });
    let Some((id, fields, is_reveal)) = target else {
        return;
    };
    if is_reveal {
        finish(Err(PromptError::Cancelled));
        return;
    }
    let values = fields
        .iter()
        .map(|field| read_text(*field))
        .collect::<Result<Vec<_>, _>>();
    let reply = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref().filter(|panel| panel.id == id)?;
        Some(values.and_then(|values| panel.prompt.reply(values)))
    });
    match reply {
        Some(Ok(reply)) => {
            // A suggested password is used only after the owner was asked if it's written down.
            let ask = ACTIVE.with(|active| {
                let mut active = active.try_borrow_mut().ok()?;
                let panel = active.as_mut().filter(|panel| panel.id == id)?;
                let suggestion = panel.suggestion.as_mut()?;
                if suggestion.acknowledged {
                    return None;
                }
                suggestion.acknowledged = true;
                Some(panel.feedback)
            });
            match ask {
                Some(feedback) => set_text(feedback, SUGGESTION_ASK),
                None => finish(Ok(reply)),
            }
        }
        Some(Err(refusal)) if current(id) => refuse(id, refusal),
        _ => (),
    }
}

/// Clears a box first and shows the message last: EN_CHANGE runs inside the clearing.
fn refuse(id: u64, refusal: Refusal) {
    let target = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut().filter(|panel| panel.id == id)?;
        let kept = refusal.focus == RefusalFocus::Keep;
        let (clear, focus) = match refusal.focus {
            RefusalFocus::Keep => (Vec::new(), None),
            RefusalFocus::ClearAndFocusNew => {
                let boxes = panel.fields.rchunks(2).next().unwrap_or_default();
                (boxes.to_vec(), boxes.first().copied())
            }
            RefusalFocus::FocusConfirm => (Vec::new(), panel.fields.last().copied()),
            RefusalFocus::ClearAndFocusRetype => {
                (panel.fields.clone(), panel.fields.first().copied())
            }
        };
        if !clear.is_empty() {
            panel.installing = true;
            panel.live_line = None;
        }
        panel.last_error = Some(refusal.message.to_string());
        panel.error_shown = true;
        Some((
            clear,
            focus,
            panel.feedback,
            kept.then(|| panel.fields.clone()),
        ))
    });
    let Some((clear, focus, feedback, kept)) = target else {
        return;
    };
    for field in clear {
        if !current(id) {
            return;
        }
        clear_box(field);
    }
    installed(id);
    // A clicked button keeps the focus unless it is moved, and it would eat what is typed next.
    let focus = focus.or_else(|| kept.and_then(|boxes| kept_refusal_focus(&boxes)));
    if let Some(focus) = focus
        && current(id)
    {
        // SAFETY: focus is a live native panel control on this thread.
        let _ = unsafe { SetFocus(Some(focus)) };
    }
    if current(id) {
        set_text(feedback, &problem_text(&refusal.message));
    }
}

/// None when the focus is already in a box: Return there stays there. Checks only the lengths.
fn kept_refusal_focus(boxes: &[HWND]) -> Option<HWND> {
    // SAFETY: GetFocus reads this thread's focused control, and each box is a live EDIT on it.
    let (focused, empty) = unsafe {
        (
            GetFocus(),
            boxes
                .iter()
                .map(|field| GetWindowTextLengthW(*field) == 0)
                .collect::<Vec<_>>(),
        )
    };
    if boxes.contains(&focused) {
        return None;
    }
    boxes.get(kept_refusal_box(&empty)?).copied()
}

/// Our own writes to the boxes are over: the next EN_CHANGE is the owner's.
fn installed(id: u64) {
    ACTIVE.with(|active| {
        if let Ok(mut active) = active.try_borrow_mut()
            && let Some(panel) = active.as_mut().filter(|panel| panel.id == id)
        {
            panel.installing = false;
        }
    });
}

/// The empty box Return should move to on the new-password panel. Checks only the length:
/// reading the text would make another copy of it.
fn first_empty_box() -> Option<HWND> {
    let fields = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. }))
            .then(|| panel.fields.clone())
    })?;
    // SAFETY: Each field is a live EDIT on this thread; only its length is read.
    fields
        .into_iter()
        .find(|field| unsafe { GetWindowTextLengthW(*field) } == 0)
}

/// Return, after its key-down passed the activity check. It moves to an empty box, then submits.
/// On a readout `submit` closes, so Return never presses Hold or Reset. It never presses Suggest
/// either: that would replace a password the owner may have just written down.
fn return_key(control_id: usize) {
    if (control_id >= FIRST_FIELD || control_id == SUGGEST)
        && let Some(field) = first_empty_box()
    {
        // SAFETY: field is a live native panel control on this thread.
        let _ = unsafe { SetFocus(Some(field)) };
        return;
    }
    submit();
}

/// Cancel or Escape, once the caller has dropped auto-repeat. Only the Recovery Kit asks first,
/// because the key on screen stops working if this step ends.
fn owner_cancel(from_key: bool) {
    let kit = ACTIVE.with(|active| {
        active
            .try_borrow()
            .ok()
            .is_some_and(|active| active.as_ref().is_some_and(|panel| panel.prompt.is_kit()))
    });
    if !kit {
        finish(Err(PromptError::Cancelled));
        return;
    }
    // Denied has already closed the panel. A blanked Kit rejects this press like any other action.
    if !activity(true) && !kit_on_hold() {
        return;
    }
    let decision = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        Some((
            panel.id,
            panel.cancel_arm.press(Instant::now(), from_key),
            panel.feedback,
            panel.fields.first().copied(),
        ))
    });
    let Some((id, decision, feedback, field)) = decision else {
        return;
    };
    match decision {
        ArmDecision::Cancel => {
            finish(Err(PromptError::Cancelled));
            return;
        }
        ArmDecision::Arm => set_text(feedback, KIT_ARMED),
        ArmDecision::Ignore => (),
    }
    // The row says "keep typing", and a clicked button keeps the focus and eats the keys. Ignore
    // too: the second half of a double-click takes the focus back.
    if let Some(field) = field
        && current(id)
    {
        // SAFETY: field is a live native panel control on this thread.
        let _ = unsafe { SetFocus(Some(field)) };
    }
}

/// True right after `activity` said no for a brief core lock on the Kit: `Hold` blanked nothing.
fn kit_on_hold() -> bool {
    ACTIVE.with(|active| {
        active.try_borrow().ok().is_some_and(|active| {
            active
                .as_ref()
                .is_some_and(|panel| panel.prompt.is_kit() && !panel.presentation.suspended)
        })
    })
}

/// True right after `activity` said no for a busy core: Denied has already closed the panel.
fn box_takes_typing(control_id: usize) -> bool {
    control_id >= FIRST_FIELD
        && ACTIVE.with(|active| {
            active.try_borrow().ok().is_some_and(|active| {
                active.as_ref().is_some_and(|panel| {
                    panel
                        .prompt
                        .box_takes_typing_while_busy(panel.presentation.suspended)
                })
            })
        })
}

/// What an EDIT takes from WM_CHAR as typing. Control codes such as Ctrl+V are not typing.
fn is_plain_typing(code: usize) -> bool {
    code == 0x08 || (code >= 0x20 && code != 0x7f)
}

/// EN_CHANGE. It also runs inside SetWindowTextW: our own writes set `installing` first, and
/// `finish` clears its boxes after taking the panel, so ACTIVE is empty for those.
fn field_edited(control_id: usize) {
    let edit = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        if panel.installing {
            return None;
        }
        let suspended = panel.presentation.suspended;
        let mut text = None;
        if !suspended && (panel.error_shown || panel.cancel_arm.armed_at.is_some()) {
            panel.error_shown = false;
            panel.last_error = None;
            text = Some(panel.normal_feedback());
        }
        panel.cancel_arm.disarm();
        let current_box = control_id == FIRST_FIELD
            && matches!(
                &panel.prompt,
                Request::Entry(Prompt::NewPassword {
                    current_password: true,
                    ..
                })
            );
        // Only an edit that is positively the Current box leaves a suggestion on screen.
        if panel.suggestion.is_some() && !current_box {
            panel.suggestion = None;
            panel.installing = true;
            panel.live_line = None;
            panel.error_shown = true;
            return Some((
                panel.id,
                panel.fields.rchunks(2).next().unwrap_or_default().to_vec(),
                panel.sensitive_labels.last().copied(),
                panel.feedback,
                (!suspended).then(|| SUGGESTION_REMOVED.to_owned()),
            ));
        }
        if !current_box && matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. })) {
            let index = control_id.checked_sub(FIRST_FIELD);
            panel
                .live
                .box_edited(Instant::now(), panel.fields.len(), index);
        }
        Some((panel.id, Vec::new(), None, panel.feedback, text))
    });
    let Some((id, boxes, label, feedback, text)) = edit else {
        return;
    };
    for field in boxes {
        if !current(id) {
            return;
        }
        clear_box(field);
    }
    if let Some(label) = label
        && current(id)
    {
        set_text(label, "");
    }
    installed(id);
    if let Some(text) = text
        && current(id)
    {
        set_text(feedback, &text);
    }
    show_recovery_progress(id);
}

/// The count line under a Recovery Key box. It reads the box but shows no character of it.
fn show_recovery_progress(id: u64) {
    let target = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref().filter(|panel| panel.id == id)?;
        let recovery = matches!(
            &panel.prompt,
            Request::Entry(Prompt::Credential { recovery: true, .. })
        );
        if !recovery || panel.presentation.suspended || panel.error_shown {
            return None;
        }
        Some((
            *panel.fields.first()?,
            panel.feedback,
            panel.prompt.feedback_text(),
        ))
    });
    let Some((field, feedback, hint)) = target else {
        return;
    };
    let Ok(value) = read_text(field) else {
        return;
    };
    let text = format!("{}. {hint}", recovery_progress(&value));
    drop(value);
    if current(id) {
        set_text(feedback, &text);
    }
}

struct ReadoutPaint {
    readout: HWND,
    viewport: HWND,
    width: i32,
    value: Zeroizing<String>,
}

fn current(id: u64) -> bool {
    ACTIVE.with(|active| {
        active
            .try_borrow()
            .ok()
            .is_some_and(|active| active.as_ref().is_some_and(|panel| panel.id == id))
    })
}

fn paint_readout(id: u64, paint: ReadoutPaint) {
    if current(id) {
        // Native setters synchronously dispatch subclass messages. The panel snapshot owns
        // its wiping text and no ACTIVE borrow survives across these user32 calls.
        set_readout(paint.readout, paint.viewport, paint.width, &paint.value);
    }
}

fn held(show: bool) {
    if show && !activity(false) {
        return;
    }
    let paint = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        let (Request::Reveal { reveal, .. }, Some(readout), Some(viewport)) =
            (&panel.prompt, panel.readout, panel.readout_viewport)
        else {
            return None;
        };
        if reveal.mode != RevealMode::Held || panel.readout_shown == show {
            return None;
        }
        panel.readout_shown = show;
        Some((
            panel.id,
            ReadoutPaint {
                readout,
                viewport,
                width: panel.readout_width,
                value: if show {
                    reveal.value.clone()
                } else {
                    Zeroizing::new(
                        "Hold Space or the button to show. Scroll to read long values.".into(),
                    )
                },
            },
        ))
    });
    if let Some((id, paint)) = paint {
        paint_readout(id, paint);
    }
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
    let target = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        Some((
            panel.id,
            panel.readout_viewport?,
            panel.readout?,
            panel.readout_width,
        ))
    });
    let Some((id, viewport, readout, width)) = target else {
        return false;
    };
    let mut info = SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_ALL,
        ..Default::default()
    };
    // SAFETY: Local SCROLLINFO has its actual size. HWNDs were copied from this panel;
    // all native calls run after dropping ACTIVE, including synchronous style/size callbacks.
    unsafe {
        if GetScrollInfo(viewport, SB_VERT, &mut info).is_err() || !current(id) {
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
        if !current(id) {
            return false;
        }
        let _ = MoveWindow(readout, 0, -next, width, (info.nMax + 1).max(112), true);
    }
    true
}

fn extend() {
    if !activity(false) {
        return;
    }
    let pending = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
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
        let authorization = hook();
        let present = ACTIVE.with(|active| {
            let Ok(mut active) = active.try_borrow_mut() else {
                return false;
            };
            let Some(panel) = active.as_mut().filter(|panel| panel.id == id) else {
                return false;
            };
            if let Request::Reveal { extend, .. } = &mut panel.prompt {
                *extend = Some(hook);
            }
            if authorization == Authorization::Allowed {
                panel.extensions += 1;
                panel.expires =
                    Some(Instant::now() + Duration::from_millis(session::ACCESSIBLE_REVEAL_MS));
            }
            true
        });
        if !present || !apply_authorization(id, authorization) {
            return;
        }
    }
    tick();
}

fn tick() {
    if !activity(false) {
        return;
    }
    revert_expired_arm();
    update_time_row();
    update_live_line();
    let pending = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        let Request::Reveal { reveal, .. } = &panel.prompt else {
            return None;
        };
        let remaining = panel
            .expires?
            .min(reveal.closes_at)
            .saturating_duration_since(Instant::now());
        Some((panel.id, panel.feedback, remaining, panel.extensions))
    });
    if let Some((id, feedback, remaining, extensions)) = pending {
        if !current(id) {
            return;
        }
        if remaining.is_zero() {
            finish(Err(PromptError::Cancelled));
        } else {
            set_text_if_changed(
                feedback,
                &timed_readout_text(remaining.as_secs() + 1, 3 - extensions),
            );
        }
    }
}

/// Ten seconds after a first Cancel on the Kit, that press no longer counts.
fn revert_expired_arm() {
    let revert = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        if !panel.cancel_arm.expired(Instant::now()) {
            return None;
        }
        panel.cancel_arm.disarm();
        Some((panel.id, panel.feedback, panel.restored_feedback()))
    });
    if let Some((id, feedback, text)) = revert
        && current(id)
    {
        set_text(feedback, &text);
    }
}

fn time_row_line(caps_lock: bool, time: &str) -> String {
    match (caps_lock, time.is_empty()) {
        (true, true) => CAPS_LOCK_TEXT.to_owned(),
        (true, false) => format!("{CAPS_LOCK_TEXT} {time}"),
        (false, _) => time.to_owned(),
    }
}

fn update_time_row() {
    // SAFETY: GetKeyState only reads keyboard state. The low bit is whether Caps Lock is on.
    let caps_lock = (unsafe { GetKeyState(0x14) } & 1) != 0;
    let row = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        // The probe only tries its locks and never counts as activity. Busy leaves the row.
        let time = panel.prompt.time_row_text(&(panel.time_left)())?;
        // Capitals change a password. They don't change a Recovery Key.
        let matters = matches!(
            &panel.prompt,
            Request::Entry(
                Prompt::Credential {
                    recovery: false,
                    ..
                } | Prompt::NewPassword { .. }
            )
        );
        Some((
            panel.id,
            panel.time_row,
            time_row_line(caps_lock && matters, &time),
        ))
    });
    if let Some((id, row, line)) = row
        && current(id)
    {
        set_text_if_changed(row, &line);
    }
}

/// New-password advice at typing pauses. It reads New and Confirm only, and approves nothing:
/// Continue always runs the real check.
fn update_live_line() {
    let now = Instant::now();
    let target = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        if !matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. }))
            || panel.suggestion.is_some()
            || panel.presentation.suspended
            || panel.error_shown
        {
            return None;
        }
        if !panel.live.poll() && !panel.live.due(now) {
            return None;
        }
        let [.., new, confirm] = panel.fields[..] else {
            return None;
        };
        Some((panel.id, new, confirm, panel.feedback))
    });
    let Some((id, new, confirm, feedback)) = target else {
        return;
    };
    let (Ok(new), Ok(confirm)) = (read_text(new), read_text(confirm)) else {
        return;
    };
    let line = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut().filter(|panel| panel.id == id)?;
        let line = panel.live.line(&new, &confirm);
        panel.live_line = Some(line.clone());
        Some(line)
    });
    if let Some(line) = line
        && current(id)
    {
        set_text(feedback, &line);
    }
}

fn suggest() {
    if !activity(false) {
        return;
    }
    let target = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        if !matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. })) {
            return None;
        }
        // EN_CHANGE fires between the two writes below, so the old one goes first.
        panel.suggestion = None;
        panel.last_error = None;
        panel.error_shown = false;
        panel.live_line = None;
        panel.installing = true;
        Some((
            panel.id,
            panel
                .fields
                .iter()
                .rev()
                .take(2)
                .copied()
                .collect::<Vec<_>>(),
            panel.sensitive_labels.last().copied(),
            panel.feedback,
        ))
    });
    let Some((id, fields, label, feedback)) = target else {
        return;
    };
    // Reversed above, so New is last.
    let new_box = fields.last().copied();
    let suggestion = policy::suggest_passphrase();
    // EDIT EN_CHANGE notifications run synchronously here, with no ACTIVE borrow held.
    for field in fields {
        if !current(id) {
            return;
        }
        set_text(field, &suggestion);
    }
    let stored = ACTIVE.with(|active| {
        let Ok(mut active) = active.try_borrow_mut() else {
            return false;
        };
        let Some(panel) = active.as_mut().filter(|panel| panel.id == id) else {
            return false;
        };
        panel.installing = false;
        panel.suggestion = Some(Suggestion {
            text: suggestion.clone(),
            acknowledged: false,
        });
        true
    });
    if !stored || !activity(false) {
        return;
    }
    if let Some(label) = label
        && current(id)
    {
        set_text(label, &suggestion);
    }
    if current(id) {
        set_text(feedback, SUGGESTED_TEXT);
    }
    // Off the button, or the next Space replaces a password the owner may have just written down.
    // An empty Current box comes first: typing there leaves the suggestion alone.
    if let Some(field) = first_empty_box().or(new_box)
        && current(id)
    {
        // SAFETY: field is a live native panel control on this thread.
        let _ = unsafe { SetFocus(Some(field)) };
    }
}

fn finish(result: Result<Reply, PromptError>) {
    let panel = ACTIVE.with(|active| active.borrow_mut().take());
    let Some(panel) = panel else {
        return;
    };
    for field in &panel.fields {
        clear_box(*field);
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
        field_edited(wparam.0 & 0xffff);
    }
    if message == WM_COMMAND && (wparam.0 >> 16) == 0 {
        if (wparam.0 & 0xffff) == CANCEL {
            owner_cancel(false);
            return LRESULT(0);
        }
        if !activity(true) {
            return LRESULT(0);
        }
        match wparam.0 & 0xffff {
            ACCEPT => submit(),
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
    if message == WM_ACTIVATE && (wparam.0 & 0xffff) == 0 {
        // Another app or window took focus: the panel closed by itself, not by the owner.
        finish(Err(PromptError::Interrupted));
    }
    if message == WM_CLOSE || message == WM_NCDESTROY {
        finish(Err(PromptError::Cancelled));
    }
    if message == WM_SIZE {
        let mut bounds = RECT::default();
        // SAFETY: The local rectangle is writable, and this callback belongs to our owner.
        if unsafe { GetClientRect(hwnd, &mut bounds) }.is_ok() {
            let resize = ACTIVE.with(|active| {
                let mut active = active.try_borrow_mut().ok()?;
                let panel = active.as_mut()?;
                let center = (bounds.right - (panel.readout_width + 24)) / 2;
                let top = panel_top(bounds.bottom, panel.panel_height);
                let across = center - panel.layout_center;
                let down = top - panel.layout_top;
                panel.layout_center = center;
                panel.layout_top = top;
                Some((panel.overlay, across, down))
            });
            if let Some((overlay, across, down)) = resize {
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
                    if across != 0 || down != 0 {
                        let mut outer = RECT::default();
                        if GetWindowRect(overlay, &mut outer).is_ok() {
                            let mut next = GetWindow(overlay, GW_CHILD).ok();
                            while let Some(child) = next {
                                let mut rect = RECT::default();
                                if GetWindowRect(child, &mut rect).is_ok() {
                                    let _ = MoveWindow(
                                        child,
                                        rect.left - outer.left + across,
                                        rect.top - outer.top + down,
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

fn handles_input(message: u32) -> bool {
    matches!(
        message,
        WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
            | WM_LBUTTONDOWN
            | WM_NCLBUTTONDOWN
            | WM_LBUTTONUP
            | WM_CAPTURECHANGED
            | WM_CANCELMODE
            | WM_KILLFOCUS
            | WM_MOUSEWHEEL
            | WM_COPY
            | WM_CUT
            | WM_CONTEXTMENU
            | WM_GETDLGCODE
    )
}

unsafe extern "system" fn input_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    // SetScrollInfo and layout setters synchronously dispatch these subclass messages.
    // Non-input messages must reach the native control without inspecting ACTIVE.
    if !handles_input(message) {
        return unsafe { DefSubclassProc(hwnd, message, wparam, lparam) };
    }
    // Bit 30 identifies an already-held key. Repeated key-downs must not defeat Auto-lock,
    // and holding Return or Escape presses nothing.
    let repeat = matches!(message, WM_KEYDOWN | WM_CHAR) && (lparam.0 & (1 << 30)) != 0;
    // SAFETY: GetDlgCtrlID reads the native child ID assigned during construction.
    let control_id = usize::try_from(unsafe { GetDlgCtrlID(hwnd) }).unwrap_or(0);
    if message == WM_KEYDOWN && (wparam.0 == 0x1b || (wparam.0 == 0x0d && control_id == CANCEL)) {
        if !repeat {
            owner_cancel(true);
        }
        return LRESULT(0);
    }
    // A busy core must not eat what is being typed in a box.
    if (matches!(message, WM_LBUTTONDOWN | WM_NCLBUTTONDOWN)
        || (matches!(message, WM_KEYDOWN | WM_CHAR) && !repeat))
        && !activity(true)
        && !(message == WM_CHAR && is_plain_typing(wparam.0) && box_takes_typing(control_id))
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
    let is_hold = control_id == HOLD || (control_id == READOUT && ACTIVE.with(|active| active.try_borrow().ok().is_some_and(|active| active.as_ref().is_some_and(|panel| matches!(&panel.prompt, Request::Reveal { reveal, .. } if reveal.mode == RevealMode::Held)))));
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
            0x0d => {
                if !repeat {
                    return_key(control_id);
                }
                return LRESULT(0);
            }
            0x09 => {
                let next = ACTIVE.with(|active| {
                    let active = active.try_borrow().ok()?;
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
    let present = ACTIVE.with(|active| {
        let Ok(mut active) = active.try_borrow_mut() else {
            return false;
        };
        let Some(panel) = active.as_mut().filter(|panel| panel.id == id) else {
            return false;
        };
        panel.activity = Some(hook);
        true
    });
    present && apply_authorization(id, authorization)
}

fn apply_authorization(id: u64, authorization: Authorization) -> bool {
    let paint = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut().filter(|panel| panel.id == id)?;
        let grace = panel.prompt.rides_out_a_blip();
        let update = panel
            .presentation
            .authorize(authorization, Instant::now(), grace);
        let mut texts = Vec::new();
        let mut readout = None;
        let mut mask = None;
        match update {
            PresentationUpdate::Clear => {
                texts.extend(
                    panel
                        .sensitive_labels
                        .iter()
                        .map(|label| (*label, Zeroizing::new(String::new()))),
                );
                if let (Some(value), Some(viewport)) = (panel.readout, panel.readout_viewport) {
                    readout = Some(ReadoutPaint {
                        readout: value,
                        viewport,
                        width: panel.readout_width,
                        value: Zeroizing::new(String::new()),
                    });
                }
                panel.readout_shown = false;
                panel.cancel_arm.disarm();
                if let Some(suggestion) = &mut panel.suggestion {
                    suggestion.acknowledged = false;
                }
                // The key is hidden for now, so its visible retyped part must be too. Hiding the
                // box instead would move focus to the overlay, where no keys work.
                if panel.prompt.shows_typing() {
                    mask = panel.fields.first().map(|field| (*field, BULLET));
                }
                texts.push((panel.feedback, Zeroizing::new("Please wait…".into())));
            }
            PresentationUpdate::Restore => {
                if let Request::Entry(Prompt::RecoveryKit { recovery, .. }) = &panel.prompt
                    && let Some(label) = panel.sensitive_labels.first()
                {
                    texts.push((*label, recovery.clone()));
                }
                if let (Request::Reveal { reveal, .. }, Some(value), Some(viewport)) =
                    (&panel.prompt, panel.readout, panel.readout_viewport)
                {
                    panel.readout_shown = reveal.mode == RevealMode::Timed
                        && panel
                            .expires
                            .is_some_and(|expires| Instant::now() < expires);
                    readout = Some(ReadoutPaint {
                        readout: value,
                        viewport,
                        width: panel.readout_width,
                        value: if panel.readout_shown {
                            reveal.value.clone()
                        } else {
                            Zeroizing::new(
                                "Hold Space or the button to show. Scroll to read long values."
                                    .into(),
                            )
                        },
                    });
                }
                if let Some(suggestion) = &panel.suggestion
                    && let Some(label) = panel.sensitive_labels.last()
                {
                    texts.push((*label, suggestion.text.clone()));
                }
                if panel.prompt.shows_typing() {
                    mask = panel.fields.first().map(|field| (*field, 0));
                }
                texts.push((panel.feedback, Zeroizing::new(panel.restored_feedback())));
            }
            _ => (),
        }
        Some((update, texts, readout, mask))
    });
    let Some((update, texts, readout, mask)) = paint else {
        return false;
    };
    if update == PresentationUpdate::Close {
        finish(Err(PromptError::Interrupted));
        return false;
    }
    if let Some((field, character)) = mask
        && current(id)
    {
        // SAFETY: This live EDIT redraws its own text; the text and the focus stay as they are.
        unsafe {
            SendMessageW(field, EM_SETPASSWORDCHAR, Some(WPARAM(character)), None);
        }
    }
    // HWNDs and wiping strings are owned snapshots. Never retain a RefCell borrow while
    // SetWindowTextW/SetScrollInfo/MoveWindow synchronously dispatch native callbacks.
    for (target, text) in texts {
        if !current(id) {
            return false;
        }
        set_text(target, &text);
    }
    if let Some(readout) = readout {
        paint_readout(id, readout);
    }
    if update == PresentationUpdate::Restore {
        show_recovery_progress(id);
    }
    current(id)
        && matches!(
            update,
            PresentationUpdate::Keep | PresentationUpdate::Restore
        )
}

#[cfg(test)]
mod tests {
    use super::{
        CAPS_LOCK_TEXT, FEEDBACK_HEIGHT, KIT_ARMED, KIT_NOTE_HEIGHT, KIT_PAPER_NOTE, Prompt,
        Request, SUGGESTED_TEXT, detail_height, handles_input, is_plain_typing, panel_height,
        panel_top, time_row_height, time_row_line,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS, WM_SIZE, WM_STYLECHANGED, WM_STYLECHANGING,
        WM_WINDOWPOSCHANGED, WM_WINDOWPOSCHANGING,
    };
    use zeroize::Zeroizing;

    #[test]
    fn a_longer_detail_gets_more_lines_up_to_the_cap() {
        assert_eq!(detail_height("", 560), 20);
        assert_eq!(detail_height(&"x".repeat(70), 560), 20);
        assert_eq!(detail_height(&"x".repeat(71), 560), 40);
        assert_eq!(detail_height("fake\nfake", 560), 40);
        assert_eq!(detail_height(&"漢".repeat(36), 560), 40);
        assert_eq!(detail_height(&"x".repeat(5_000), 560), 216);
    }

    #[test]
    fn the_fixed_rows_hold_their_longest_text_by_this_estimate() {
        assert!(detail_height(KIT_PAPER_NOTE, 560) <= KIT_NOTE_HEIGHT);
        assert!(detail_height(KIT_ARMED, 560) <= FEEDBACK_HEIGHT);
        assert!(detail_height(SUGGESTED_TEXT, 560) <= FEEDBACK_HEIGHT);
    }

    #[test]
    fn panels_center_but_keep_their_title_in_view() {
        assert_eq!(panel_top(1000, 400), 300);
        assert_eq!(panel_top(456, 400), 28);
        assert_eq!(panel_top(300, 400), 28);
    }

    #[test]
    fn the_fullest_panels_fit_the_smallest_window() {
        let new_password = Request::Entry(Prompt::NewPassword {
            title: "Fake".into(),
            detail: "fake ".repeat(47),
            current_password: true,
        });
        let kit = Request::Entry(Prompt::RecoveryKit {
            title: "Fake".into(),
            detail: "fake ".repeat(68),
            recovery: Zeroizing::new("FAKE-FAKE-FAKE-FAKE-FAKE-FAKE-FAKE".into()),
            rotation: true,
        });
        assert_eq!(time_row_height(&new_password, 560), 40);
        assert_eq!(time_row_height(&kit, 560), 60);
        for request in [new_password, kit] {
            let (_, detail) = request.text();
            let height = panel_height(
                &request,
                detail_height(detail, 560),
                time_row_height(&request, 560),
            );
            assert!(28 + height <= 560, "{height}");
        }
    }

    #[test]
    fn caps_lock_comes_before_the_time() {
        assert_eq!(time_row_line(false, ""), "");
        assert_eq!(time_row_line(false, "Fake time."), "Fake time.");
        assert_eq!(time_row_line(true, ""), CAPS_LOCK_TEXT);
        assert_eq!(
            time_row_line(true, "Fake time."),
            format!("{CAPS_LOCK_TEXT} Fake time.")
        );
    }

    #[test]
    fn only_typing_passes_while_the_core_is_busy() {
        for code in [b'a' as usize, b' ' as usize, 0x08, 0x6f22] {
            assert!(is_plain_typing(code));
        }
        // Return, Tab, Escape, Ctrl+V and Delete.
        for code in [13, 9, 27, 0x16, 0x7f] {
            assert!(!is_plain_typing(code));
        }
    }

    #[test]
    fn native_layout_reentry_bypasses_panel_input_policy() {
        for message in [
            WM_SIZE,
            WM_STYLECHANGING,
            WM_STYLECHANGED,
            WM_WINDOWPOSCHANGING,
            WM_WINDOWPOSCHANGED,
        ] {
            assert!(!handles_input(message));
        }
        for message in [WM_CHAR, WM_KEYDOWN, WM_KILLFOCUS] {
            assert!(handles_input(message));
        }
    }
}
