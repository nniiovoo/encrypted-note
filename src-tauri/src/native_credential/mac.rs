//! AppKit controls replace the content view of the already protected NSWindow. The original
//! WebView stays retained, but cannot receive input or draw over the native credential panel.
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{ClassType, MainThreadOnly, class, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezelStyle, NSButton, NSButtonType, NSColor, NSFont, NSResponder,
    NSScrollView, NSSecureTextField, NSTextField, NSTextFieldCell, NSTextView, NSView, NSWindow,
};
use objc2_foundation::{
    MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoopCommonModes,
    NSSize, NSString, ns_string,
};
use zeroize::Zeroizing;

use super::{
    Activity, ArmDecision, Authorization, CAPS_LOCK_TEXT, Callback, CancelArm, HOLD_TO_SHOW_LABEL,
    KIT_ARMED, KIT_PAPER_NOTE, LiveCheck, MayShow, PresentationState, PresentationUpdate, Prompt,
    PromptError, RESET_BUTTON, RefusalFocus, Reply, Request, RevealMode, SHOW_REFUSED_TEXT,
    SHOW_TYPING_LABEL, SUGGEST_BUTTON, SUGGESTED_TEXT, SUGGESTION_ASK, SUGGESTION_REMOVED,
    ShowCheck, Suggestion, TimeLeft, TimeLimit, TimeProbe, kept_refusal_box, problem_text,
    readout_text, recovery_mirror, recovery_progress, timed_readout_text,
};

/// Long enough for the fullest Wallet warning, short enough to leave the rows below it on screen.
const DETAIL_MAX_HEIGHT: f64 = 216.0;
const HOLD_LIMIT: Duration = Duration::from_secs(10);
/// Show what I type turns itself off after this long with no real key press.
const TYPED_IDLE_LIMIT: Duration = Duration::from_secs(60);

thread_local! {
    static ACTIVE: RefCell<Option<Panel>> = const { RefCell::new(None) };
    /// True while this file changes a box's text itself: AppKit's notice of that isn't an edit.
    static SETTING_BOXES: Cell<bool> = const { Cell::new(false) };
    /// The retype box's own field editor. Here, not in the Panel, because AppKit asks for it from
    /// inside other calls. `finish` blanks and drops it.
    static RETYPE_EDITOR: RefCell<Option<Retained<RetypeEditor>>> = const { RefCell::new(None) };
}

struct Panel {
    id: u64,
    window: Retained<NSWindow>,
    original: Retained<NSView>,
    original_focus: Option<Retained<NSResponder>>,
    _view: Retained<NSView>,
    _target: Retained<Target>,
    _authentication: Option<Retained<NSView>>,
    fields: Vec<Retained<NSTextField>>,
    sensitive_labels: Vec<Retained<NSTextField>>,
    feedback: Retained<NSTextField>,
    time_row: Retained<NSTextField>,
    time_row_shown: String,
    /// The probe's last answer: a busy core gives none, and Caps Lock can change without one.
    time_text: String,
    prompt: Request,
    completion: Callback,
    activity: Option<Activity>,
    time_left: TimeProbe,
    may_show: MayShow,
    presentation: PresentationState,
    cancel_arm: CancelArm,
    last_error: Option<String>,
    /// A refusal or the removed-suggestion notice is on screen. It stays until the next edit.
    error_shown: bool,
    live: LiveCheck,
    /// Counts and yes/no answers only. Comes back after a brief suspension.
    live_line: Option<String>,
    suggestion: Option<Suggestion>,
    show_typing: Option<Retained<NSButton>>,
    typed_hold: Option<Retained<HoldButton>>,
    /// A snapshot while held. Not in `sensitive_labels`, so Restore can never refill it.
    hold_mirror: Option<Retained<NSTextField>>,
    /// Which box the hold mirror shows: that box's own label, never anything typed.
    hold_caption: Option<Retained<NSTextField>>,
    hold_until: Option<Instant>,
    /// The New or Confirm box typed in last, by index. Never Current. A hold falls back on it.
    last_edited: Option<usize>,
    /// New and Confirm were empty when last read. Tracked so the timer never reads the boxes.
    boxes_empty: bool,
    /// The last clear answer from `may_show`. Starts as no, so a busy first frame stays a no.
    show_usable: bool,
    /// Why the two controls are grey. No secret, so it isn't in `sensitive_labels`.
    show_refused: Option<Retained<NSTextField>>,
    /// The last real key press while Show what I type is ticked. None while it isn't.
    typed_key_at: Option<Instant>,
    /// A key landed in a blip, so the count and mirror wait for the next good check.
    lines_stale: bool,
    readout: Option<Retained<NSView>>,
    readout_scroll: Option<Retained<ReadoutScroll>>,
    readout_shown: bool,
    expires: Option<Instant>,
    extensions: u8,
    timer: Option<Retained<AnyObject>>,
}

// All callbacks run on AppKit's main thread. The target is retained until controls are removed.
define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "EncryptedNoteCredentialTarget"]
    struct Target;
    // SAFETY: NSObjectProtocol adds no requirements.
    unsafe impl NSObjectProtocol for Target {}
    impl Target {
        #[unsafe(method(submit:))]
        fn submit(&self, _sender: Option<&AnyObject>) {
            // The Return key equivalent gets here too. Holding Return must not submit again.
            if !current_event_repeats() && activity(true) { submit(); }
        }
        #[unsafe(method(cancel:))]
        fn cancel(&self, _sender: Option<&AnyObject>) { owner_cancel(); }
        #[unsafe(method(suggest:))]
        fn suggest(&self, sender: Option<&AnyObject>) { if activity(true) { suggest(sender); } }
        #[unsafe(method(extend:))]
        fn extend(&self, _sender: Option<&AnyObject>) { if activity(true) { extend(); } }
        #[unsafe(method(showTyping:))]
        fn show_typing(&self, _sender: Option<&AnyObject>) { show_typing_changed(); }
        #[unsafe(method(tick:))]
        fn tick(&self, _sender: Option<&AnyObject>) { tick(); }
        #[unsafe(method(focusLost:))]
        fn focus_lost(&self, _notification: &AnyObject) { finish(Err(PromptError::Interrupted)); }
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn command(&self, _control: &AnyObject, _editor: &AnyObject, command: Sel) -> bool {
            if command == sel!(cancelOperation:) { owner_cancel(); true }
            else if command == sel!(insertNewline:) { return_pressed(); true }
            else { false }
        }
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, notification: &AnyObject) { text_changed(notification); }
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn editing(&self, _notification: &AnyObject) {
            // A fallback: `present` already did this for the first box.
            let boxes = ACTIVE.with(|active| {
                let active = active.try_borrow().ok()?;
                let panel = active.as_ref()?;
                Some((panel.fields.clone(), panel.prompt.shows_typing()))
            });
            if let Some((fields, roman_only)) = boxes {
                for field in &fields {
                    quiet_editor(field, roman_only);
                }
            }
        }
    }
);

define_class!(
    #[unsafe(super = NSButton)]
    #[thread_kind = MainThreadOnly]
    #[name = "EncryptedNoteCredentialHoldButton"]
    struct HoldButton;
    unsafe impl NSObjectProtocol for HoldButton {}
    impl HoldButton {
        #[unsafe(method(canBecomeKeyView))]
        fn can_become_key_view(&self) -> bool { self.isEnabled() }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &AnyObject) {
            if !self.isEnabled() || !activity(true) { return; }
            self.highlight(true);
            held(true);
        }
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &AnyObject) {
            self.highlight(false);
            held(false);
            return_hold_focus(self);
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &AnyObject) {
            // SAFETY: keyDown: receives an NSEvent and this is its documented BOOL property.
            let repeated: bool = unsafe { msg_send![event, isARepeat] };
            if !activity(!repeated) { return; }
            if typed_hold_key(event, repeated) { return; }
            if reveal_navigation(event) { return; }
            // SAFETY: keyDown: receives an NSEvent and this is its documented string property.
            let text: Option<Retained<NSString>> = unsafe { msg_send![event, charactersIgnoringModifiers] };
            if text.is_some_and(|text| text.to_string() == " ") { held(true); }
            // SAFETY: Forward other navigation keys to NSButton's own implementation.
            else { unsafe { let _: () = msg_send![super(self), keyDown: event]; } }
        }
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &AnyObject) {
            let passed = activity(true);
            // Whatever the check says: a release must always end what the press showed.
            if is_space_event(event) {
                held(false);
                return_hold_focus(self);
            }
            if !passed { return; }
            // SAFETY: Forward the unmodified live NSEvent through the native button.
            unsafe { let _: () = msg_send![super(self), keyUp: event]; }
        }
        #[unsafe(method(resignFirstResponder))]
        fn resign(&self) -> bool {
            held(false);
            // SAFETY: NSResponder's documented no-argument BOOL method.
            unsafe { msg_send![super(self), resignFirstResponder] }
        }
    }
);

// With Keyboard navigation off, the Mac's default, Tab skips a plain button and tickbox. Cancel and
// Continue stay plain: Escape and Return already reach them.
define_class!(
    #[unsafe(super = NSButton)]
    #[thread_kind = MainThreadOnly]
    #[name = "EncryptedNoteCredentialKeyButton"]
    struct KeyButton;
    unsafe impl NSObjectProtocol for KeyButton {}
    impl KeyButton {
        #[unsafe(method(canBecomeKeyView))]
        fn can_become_key_view(&self) -> bool { self.isEnabled() }
    }
);

// The Recovery Kit's retype box isn't a secure field, so AppKit would let its text reach the
// ordinary clipboard and system text tools. It gets its own field editor that sends nothing out.
// Typing, paste and Select All stay. tao owns the window delegate, so the cell supplies it.
define_class!(
    #[unsafe(super = NSTextView)]
    #[thread_kind = MainThreadOnly]
    #[name = "EncryptedNoteCredentialRetypeEditor"]
    struct RetypeEditor;
    unsafe impl NSObjectProtocol for RetypeEditor {}
    impl RetypeEditor {
        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {}
        #[unsafe(method(cut:))]
        fn cut(&self, _sender: Option<&AnyObject>) {}
        #[unsafe(method(validateMenuItem:))]
        fn validate_menu_item(&self, item: &AnyObject) -> bool {
            // SAFETY: AppKit passes a live NSMenuItem, and action is its documented property.
            let action: Option<Sel> = unsafe { msg_send![item, action] };
            if action.is_some_and(|action| action == sel!(copy:) || action == sel!(cut:)) {
                false
            } else {
                // SAFETY: Forward the unchanged menu item to NSTextView's own check.
                unsafe { msg_send![super(self), validateMenuItem: item] }
            }
        }
        #[unsafe(method_id(menuForEvent:))]
        fn menu_for_event(&self, _event: &AnyObject) -> Option<Retained<AnyObject>> { None }
        // The app menu's Services asks here. A return type alone only brings text in.
        #[unsafe(method_id(validRequestorForSendType:returnType:))]
        fn valid_requestor(
            &self,
            send: Option<&AnyObject>,
            take: Option<&AnyObject>,
        ) -> Option<Retained<AnyObject>> {
            if send.is_some() {
                None
            } else {
                // SAFETY: Forward the unchanged pasteboard types to NSTextView's own check.
                unsafe { msg_send![super(self), validRequestorForSendType: send, returnType: take] }
            }
        }
        #[unsafe(method(dragSelectionWithEvent:offset:slideBack:))]
        fn drag_selection(&self, _event: &AnyObject, _offset: NSSize, _slide_back: bool) -> bool {
            false
        }
        #[unsafe(method_id(writablePasteboardTypes))]
        fn writable_pasteboard_types(&self) -> Retained<NSArray<NSString>> { NSArray::new() }
        // Writing Tools off (NSWritingToolsBehaviorNone). A getter, because AppKit sets this again
        // each time the box takes focus. Older macOS, which has no such property, never asks.
        #[unsafe(method(writingToolsBehavior))]
        fn writing_tools_behavior(&self) -> isize { -1 }
        #[unsafe(method(setWritingToolsBehavior:))]
        fn set_writing_tools_behavior(&self, _behavior: isize) {}
        // Look Up would put this text in a system window that Capture Hiding doesn't cover.
        #[unsafe(method(quickLookWithEvent:))]
        fn quick_look_with_event(&self, _event: &AnyObject) {}
        #[unsafe(method(quickLookPreviewItems:))]
        fn quick_look_preview_items(&self, _sender: Option<&AnyObject>) {}
        // A force click's own route to Look Up. NSSecureTextView closes it with these two. They are
        // private names: if Apple changes them, only the physical check would notice.
        #[unsafe(method_id(_immediateActionAnimationControllerForCharacterAtIndex:withRecognizer:))]
        fn immediate_action_controller(
            &self,
            _index: usize,
            _recognizer: Option<&AnyObject>,
        ) -> Option<Retained<AnyObject>> {
            None
        }
        #[unsafe(method_id(_revealAnimationControllerForCharacterAtIndex:))]
        fn reveal_controller(&self, _index: usize) -> Option<Retained<AnyObject>> { None }
    }
);

define_class!(
    #[unsafe(super = NSTextFieldCell)]
    #[thread_kind = MainThreadOnly]
    #[name = "EncryptedNoteCredentialRetypeCell"]
    struct RetypeCell;
    unsafe impl NSObjectProtocol for RetypeCell {}
    impl RetypeCell {
        #[unsafe(method_id(fieldEditorForView:))]
        fn field_editor_for_view(&self, _view: &AnyObject) -> Option<Retained<RetypeEditor>> {
            let mtm = self.mtm();
            RETYPE_EDITOR.with(|editor| {
                let mut editor = editor.try_borrow_mut().ok()?;
                Some(editor.get_or_insert_with(|| new_retype_editor(mtm)).clone())
            })
        }
    }
);

define_class!(
    #[unsafe(super = NSTextField)]
    #[thread_kind = MainThreadOnly]
    #[name = "EncryptedNoteCredentialRetypeField"]
    struct RetypeField;
    unsafe impl NSObjectProtocol for RetypeField {}
    impl RetypeField {
        // A new field makes this cell with a plain field's settings.
        #[unsafe(method(cellClass))]
        fn cell_class() -> &'static AnyClass { RetypeCell::class() }
    }
);

fn new_retype_editor(mtm: MainThreadMarker) -> Retained<RetypeEditor> {
    // SAFETY: NSTextView's documented initializer on a fresh allocation of this subclass, then its
    // documented setters.
    unsafe {
        let editor: Retained<RetypeEditor> =
            msg_send![RetypeEditor::alloc(mtm), initWithFrame: rect(0.0, 0.0, 0.0, 0.0)];
        let _: () = msg_send![&editor, setFieldEditor: true];
        let _: () = msg_send![&editor, setRichText: false];
        let _: () = msg_send![&editor, setImportsGraphics: false];
        // Some macOS versions add a force-click recognizer at init. Typing and clicks don't use it.
        let _: () = msg_send![&editor, setGestureRecognizers: &*NSArray::<AnyObject>::new()];
        editor
    }
}

impl Target {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        // SAFETY: NSObject init has no additional requirements.
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

define_class!(
    #[unsafe(super = NSScrollView)]
    #[thread_kind = MainThreadOnly]
    #[name = "EncryptedNoteCredentialReadoutScroll"]
    struct ReadoutScroll;
    unsafe impl NSObjectProtocol for ReadoutScroll {}
    impl ReadoutScroll {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool { true }
        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &AnyObject) {
            if !activity(true) { return; }
            // SAFETY: Forward the unchanged native NSEvent to NSScrollView's own scrolling.
            unsafe { let _: () = msg_send![super(self), scrollWheel: event]; }
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &AnyObject) {
            if !activity(!unsafe { msg_send![event, isARepeat] }) { return; }
            if reveal_navigation(event) { return; }
            // SAFETY: Forward an unmodified native key event to the inherited responder.
            unsafe { let _: () = msg_send![super(self), keyDown: event]; }
        }
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &AnyObject) {
            if is_space_event(event) { held(false); }
            // SAFETY: Forward the unmodified native event to NSScrollView.
            unsafe { let _: () = msg_send![super(self), keyUp: event]; }
        }
        #[unsafe(method(resignFirstResponder))]
        fn resign(&self) -> bool {
            held(false);
            // SAFETY: NSResponder's documented no-argument BOOL method.
            unsafe { msg_send![super(self), resignFirstResponder] }
        }
    }
);

pub(super) fn present(
    window: &tauri::WebviewWindow,
    id: u64,
    prompt: Request,
    on_activity: Activity,
    time_left: TimeProbe,
    may_show: MayShow,
    completion: Callback,
) -> Result<(), PromptError> {
    let mtm = MainThreadMarker::new().ok_or(PromptError::Unavailable)?;
    if ACTIVE.with(|active| active.borrow().is_some()) {
        return Err(PromptError::Busy);
    }
    let pointer = window
        .ns_window()
        .map_err(|_| PromptError::Unavailable)?
        .cast::<NSWindow>();
    // SAFETY: Tauri owns this live main NSWindow; UI-thread use is checked above. Retaining
    // keeps it alive until the native panel has been removed, including cancellation.
    let window = unsafe { Retained::retain(pointer) }.ok_or(PromptError::Unavailable)?;
    let original = window.contentView().ok_or(PromptError::Unavailable)?;
    if !window.isKeyWindow() {
        return Err(PromptError::Unavailable);
    }
    let original_focus = window.firstResponder();
    let root = NSView::initWithFrame(NSView::alloc(mtm), original.bounds());
    root.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let target = Target::new(mtm);
    let bounds = root.bounds();
    // Rows go downward from 0 in this full-width container. Its real height is known only after
    // the last row, so it gets that height, and its place in the window, at the end.
    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, bounds.size.width, 0.0));
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable
            | NSAutoresizingMaskOptions::ViewMinYMargin
            | NSAutoresizingMaskOptions::ViewMaxYMargin,
    );
    view.setAutoresizesSubviews(false);
    let width = (bounds.size.width - 64.0).clamp(240.0, 560.0);
    let x = (bounds.size.width - width) / 2.0;
    let mut y = 0.0;
    let (title, detail) = prompt.text();
    let heading = label(&view, title, x, place(&mut y, 30.0, 8.0), width, 30.0, mtm);
    heading.setFont(Some(&NSFont::boldSystemFontOfSize(21.0)));
    let detail_label = label(&view, detail, x, 0.0, width, 0.0, mtm);
    let detail_height = fitted_height(&detail_label, width).min(DETAIL_MAX_HEIGHT);
    detail_label.setFrame(rect(
        x,
        place(&mut y, detail_height, 8.0),
        width,
        detail_height,
    ));
    let mut authentication = None;
    if let Request::Biometric(biometric) = &prompt {
        let pointer = crate::biometry::authentication_view(biometric.request).cast::<NSView>();
        // SAFETY: Swift returns a +1 retained LAAuthenticationView (an NSView subclass) made on
        // this main thread for exactly this request, or null. Retained takes over that one
        // reference and releases it with the Panel.
        let embedded = unsafe { Retained::from_raw(pointer) }.ok_or(PromptError::Unavailable)?;
        // SAFETY: NSView's documented size and accessibility accessors on a live view.
        let size: NSSize = unsafe {
            let _: () = msg_send![&embedded, setAccessibilityLabel: ns_string!("Touch ID")];
            msg_send![&embedded, fittingSize]
        };
        let (side_w, side_h) = if size.width > 0.0 && size.height > 0.0 {
            (size.width, size.height)
        } else {
            (64.0, 64.0)
        };
        embedded.setFrame(rect(
            (bounds.size.width - side_w) / 2.0,
            place(&mut y, side_h, 12.0),
            side_w,
            side_h,
        ));
        anchor(&embedded);
        view.addSubview(&embedded);
        authentication = Some(embedded);
    }
    let mut sensitive_labels = Vec::new();
    let mut readout = None;
    let mut readout_scroll = None;
    let mut readout_shown = false;
    let mut hold_button = None;
    let mut extend_button = None;
    if let Request::Reveal { reveal, .. } = &prompt {
        // SAFETY: Native AppKit initializers; NSTextView is an NSView subclass. Configure
        // read-only/nonselectable before installation, with no field editor or undo history.
        let (scroll, value): (Retained<ReadoutScroll>, Retained<NSView>) = unsafe {
            let scroll = msg_send![ReadoutScroll::alloc(mtm), initWithFrame: rect(x, place(&mut y, 112.0, 12.0), width, 112.0)];
            let allocated: *mut AnyObject = msg_send![class!(NSTextView), alloc];
            let value: *mut NSView =
                msg_send![allocated, initWithFrame: rect(0.0, 0.0, width - 20.0, 112.0)];
            let value = Retained::from_raw(value).ok_or(PromptError::Unavailable)?;
            let _: () = msg_send![&value, setEditable: false];
            let _: () = msg_send![&value, setSelectable: false];
            let _: () = msg_send![&value, setRichText: false];
            let _: () = msg_send![&value, setAllowsUndo: false];
            let _: () = msg_send![&value, setImportsGraphics: false];
            let _: () = msg_send![&value, setVerticallyResizable: false];
            let _: () = msg_send![&value, setHorizontallyResizable: false];
            let _: () = msg_send![&value, setTextContainerInset: NSSize::new(4.0, 4.0)];
            let container: Option<Retained<AnyObject>> = msg_send![&value, textContainer];
            if let Some(container) = container {
                let _: () = msg_send![&container, setWidthTracksTextView: true];
                let _: () = msg_send![&container, setHeightTracksTextView: false];
                let _: () = msg_send![&container, setContainerSize: NSSize::new(width - 20.0, 100_000_000.0)];
            }
            let _: () =
                msg_send![&value, setFont: &*NSFont::monospacedSystemFontOfSize_weight(16.0, 0.0)];
            let _: () = msg_send![&value, unregisterDraggedTypes];
            (scroll, value)
        };
        anchor(&scroll);
        scroll.setHasVerticalScroller(true);
        scroll.setAutohidesScrollers(true);
        scroll.setDocumentView(Some(&value));
        view.addSubview(&scroll);
        readout_shown = reveal.mode == RevealMode::Timed;
        set_readout(
            &value,
            &scroll,
            if readout_shown {
                &reveal.value
            } else {
                "Hold Space or the button to show. Scroll to read long values."
            },
        );
        readout = Some(value);
        readout_scroll = Some(scroll);
        let button_y = place(&mut y, 32.0, 12.0);
        if reveal.mode == RevealMode::Held {
            hold_button = Some(new_hold_button(
                &view,
                "Hold to show (or hold Space)",
                rect(x, button_y, 230.0, 32.0),
                mtm,
            ));
        } else {
            let extend = key_button(
                &view,
                RESET_BUTTON,
                rect(x, button_y, 200.0, 30.0),
                &target,
                sel!(extend:),
                mtm,
            );
            extend.setBezelStyle(NSBezelStyle::Push);
            extend_button = Some(extend);
        }
    }
    if let Request::Entry(Prompt::RecoveryKit { recovery, .. }) = &prompt {
        label(
            &view,
            KIT_PAPER_NOTE,
            x,
            place(&mut y, 48.0, 6.0),
            width,
            48.0,
            mtm,
        );
        let key = label(
            &view,
            recovery,
            x,
            place(&mut y, 32.0, 8.0),
            width,
            32.0,
            mtm,
        );
        key.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(20.0, 0.0)));
        sensitive_labels.push(key);
    }
    let mut fields = Vec::new();
    for caption in prompt.labels() {
        label(
            &view,
            caption,
            x,
            place(&mut y, 22.0, 2.0),
            width,
            22.0,
            mtm,
        );
        let frame = rect(x, place(&mut y, 28.0, 8.0), width, 28.0);
        let field: Retained<NSTextField> = if prompt.shows_typing() {
            // SAFETY: NSTextField's documented initializer on a fresh allocation of this subclass.
            let field: Retained<RetypeField> =
                unsafe { msg_send![RetypeField::alloc(mtm), initWithFrame: frame] };
            let field = Retained::into_super(field);
            field.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(16.0, 0.0)));
            field.setPlaceholderString(Some(ns_string!("XXXX-XXXX")));
            field
        } else {
            let field = NSSecureTextField::initWithFrame(NSSecureTextField::alloc(mtm), frame);
            field.setFont(Some(&NSFont::systemFontOfSize(15.0)));
            Retained::into_super(field)
        };
        anchor(&field);
        // SAFETY: The retained target implements these exact NSTextField delegate methods and
        // outlives the native controls. Accessibility labels contain action text, never input.
        unsafe {
            let _: () = msg_send![&field, setDelegate: &*target];
            let _: () = msg_send![&field, setAccessibilityLabel: &*NSString::from_str(caption)];
            let _: () = msg_send![&field, setAllowsEditingTextAttributes: false];
            let _: () = msg_send![&field, unregisterDraggedTypes];
            view.addSubview(&field);
        }
        fields.push(field);
    }
    let mut show_typing = None;
    let mut show_refused = None;
    if prompt.offers_show_typing() {
        // The box stays secure. The tickbox and the separate read-only line share one row.
        let row_y = place(&mut y, 24.0, 6.0);
        let checkbox = key_button(
            &view,
            SHOW_TYPING_LABEL,
            rect(x, row_y, 150.0, 24.0),
            &target,
            sel!(showTyping:),
            mtm,
        );
        // The settings of NSButton's own checkbox initializer, which can't make a subclass.
        checkbox.setButtonType(NSButtonType::Switch);
        checkbox.setBezelStyle(NSBezelStyle::FlexiblePush);
        sensitive_labels.push(typed_line(
            &view,
            rect(x + 156.0, row_y, width - 156.0, 24.0),
            mtm,
        ));
        show_typing = Some(Retained::into_super(checkbox));
        let refused = label(&view, SHOW_REFUSED_TEXT, x, 0.0, width, 36.0, mtm);
        refused.setFrame(rect(x, place(&mut y, 36.0, 4.0), width, 36.0));
        refused.setHidden(true);
        show_refused = Some(refused);
    }
    let mut suggestion_button = None;
    let mut typed_hold = None;
    let mut hold_mirror = None;
    let mut hold_caption = None;
    if matches!(&prompt, Request::Entry(Prompt::NewPassword { .. })) {
        let row_y = place(&mut y, 30.0, 4.0);
        let suggest = key_button(
            &view,
            SUGGEST_BUTTON,
            rect(x, row_y, 170.0, 30.0),
            &target,
            sel!(suggest:),
            mtm,
        );
        suggest.setBezelStyle(NSBezelStyle::Push);
        suggestion_button = Some(suggest);
        typed_hold = prompt.offers_hold_to_show().then(|| {
            new_hold_button(
                &view,
                HOLD_TO_SHOW_LABEL,
                rect(x + 178.0, row_y, 230.0, 30.0),
                mtm,
            )
        });
        hold_caption = Some(label(
            &view,
            "",
            x,
            place(&mut y, 18.0, 0.0),
            width,
            18.0,
            mtm,
        ));
        let row_y = place(&mut y, 48.0, 4.0);
        let suggestion = label(&view, "", x, row_y, width, 48.0, mtm);
        suggestion.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(16.0, 0.0)));
        sensitive_labels.push(suggestion);
        // The same row: a hold is refused while a suggestion is on screen.
        hold_mirror = Some(typed_line(&view, rect(x, row_y, width, 48.0), mtm));
        // The same row again: nothing typed shows while this does, and it waits for a suggestion.
        let refused = label(&view, SHOW_REFUSED_TEXT, x, row_y, width, 48.0, mtm);
        refused.setHidden(true);
        show_refused = Some(refused);
    }
    let feedback = label(
        &view,
        &if prompt.offers_show_typing() {
            recovery_feedback(&prompt, "")
        } else {
            prompt.feedback_text()
        },
        x,
        place(&mut y, 36.0, 6.0),
        width,
        36.0,
        mtm,
    );
    let time_row = label(&view, "", x, 0.0, width, 0.0, mtm);
    let time_height = time_row_height(&prompt, &time_row, width);
    time_row.setFrame(rect(x, place(&mut y, time_height, 4.0), width, time_height));
    let button_y = place(&mut y, 30.0, 0.0);
    let is_reveal = matches!(&prompt, Request::Reveal { .. });
    // A readout has one button, which keeps Escape. Nothing here hints that a copy can be undone.
    let cancel = button(
        &view,
        if is_reveal { "Close" } else { "Cancel" },
        if is_reveal {
            rect(x + width - 112.0, button_y, 112.0, 30.0)
        } else {
            rect(x, button_y, 100.0, 30.0)
        },
        &target,
        sel!(cancel:),
        mtm,
    );
    cancel.setKeyEquivalent(ns_string!("\u{1b}"));
    // Touch ID has nothing to submit: the sensor is the only way forward.
    let accept = matches!(&prompt, Request::Entry(_)).then(|| {
        let accept = button(
            &view,
            "Continue",
            rect(x + width - 112.0, button_y, 112.0, 30.0),
            &target,
            sel!(submit:),
            mtm,
        );
        accept.setKeyEquivalent(ns_string!("\r"));
        accept
    });
    let mut key_views: Vec<&NSView> = Vec::new();
    for field in &fields {
        key_views.push(&**field);
    }
    if let Some(checkbox) = &show_typing {
        key_views.push(&**checkbox);
    }
    if let Some(scroll) = &readout_scroll {
        key_views.push(&**scroll);
    }
    if let Some(button) = &suggestion_button {
        key_views.push(&**button);
    }
    if let Some(button) = &typed_hold {
        key_views.push(&**button);
    }
    if let Some(button) = &hold_button {
        key_views.push(&**button);
    }
    if let Some(button) = &extend_button {
        key_views.push(&**button);
    }
    key_views.push(&*cancel);
    if let Some(accept) = &accept {
        key_views.push(&**accept);
    }
    // SAFETY: All key views are retained by the native view/Panel for the complete key loop.
    unsafe {
        for pair in key_views.windows(2) {
            pair[0].setNextKeyView(Some(pair[1]));
        }
        if let (Some(first), Some(last)) = (key_views.first(), key_views.last()) {
            last.setNextKeyView(Some(first));
        }
    }
    // Now the rows are placed: center the group, with room for the Overlay title bar. The bounds
    // origin puts row 0 at the container's top; subviews must not autoresize during this.
    let height = -y;
    let top = ((bounds.size.height - height) / 2.0).max(28.0);
    view.setFrame(rect(
        0.0,
        bounds.size.height - top - height,
        bounds.size.width,
        height,
    ));
    view.setBoundsOrigin(NSPoint::new(0.0, -height));
    view.setAutoresizesSubviews(true);
    root.addSubview(&view);
    let expires =
        matches!(&prompt, Request::Reveal { reveal, .. } if reveal.mode == RevealMode::Timed)
            .then(|| Instant::now() + Duration::from_millis(session::ACCESSIBLE_REVEAL_MS));
    // SAFETY: Notification selectors have the documented one-object signature. The observer
    // is removed before target release, and timer callbacks remain on this AppKit run loop.
    let timer = unsafe {
        let center: Retained<AnyObject> = msg_send![class!(NSNotificationCenter), defaultCenter];
        let _: () = msg_send![&center, addObserver: &*target, selector: sel!(focusLost:), name: ns_string!("NSWindowDidResignKeyNotification"), object: &*window];
        let timer: Retained<AnyObject> = msg_send![class!(NSTimer), scheduledTimerWithTimeInterval: 0.2f64, target: &*target, selector: sel!(tick:), userInfo: Option::<&AnyObject>::None, repeats: true];
        // Track deadlines/Auto-lock even inside NSScrollView scrollbar tracking loops.
        let run_loop: Retained<AnyObject> = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![&run_loop, addTimer: &*timer, forMode: NSRunLoopCommonModes];
        Some(timer)
    };
    ACTIVE.with(|active| {
        *active.borrow_mut() = Some(Panel {
            id,
            window: window.clone(),
            original,
            original_focus,
            _view: root.clone(),
            _target: target,
            _authentication: authentication,
            fields,
            sensitive_labels,
            feedback,
            time_row,
            time_row_shown: String::new(),
            time_text: String::new(),
            prompt,
            completion,
            activity: Some(on_activity),
            time_left,
            may_show,
            presentation: PresentationState::default(),
            cancel_arm: CancelArm::default(),
            last_error: None,
            error_shown: false,
            live: LiveCheck::default(),
            live_line: None,
            suggestion: None,
            show_typing,
            typed_hold,
            hold_mirror,
            hold_caption,
            hold_until: None,
            last_edited: None,
            boxes_empty: true,
            show_usable: false,
            show_refused,
            typed_key_at: None,
            lines_stale: false,
            readout,
            readout_scroll,
            readout_shown,
            expires,
            extensions: 0,
            timer,
        })
    });
    activity(false);
    if !ACTIVE.with(|active| active.borrow().as_ref().is_some_and(|panel| panel.id == id)) {
        return Err(PromptError::Cancelled);
    }
    update_show_controls();
    ACTIVE.with(|active| {
        if let Some(panel) = active.borrow_mut().as_mut() {
            update_time_row(panel);
        }
    });
    window.setContentView(Some(&root));
    let first = ACTIVE.with(|active| {
        let active = active.borrow();
        let panel = active.as_ref()?;
        Some((panel.fields.first()?.clone(), panel.prompt.shows_typing()))
    });
    if let Some((field, roman_only)) = first {
        window.makeFirstResponder(Some(&field));
        // Now, not at the first edit: the first letter must already arrive as typed.
        quiet_editor(&field, roman_only);
    }
    if let Some(button) = &hold_button {
        window.makeFirstResponder(Some(button));
    } else if let Some(scroll) = ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .and_then(|panel| panel.readout_scroll.clone())
    }) {
        window.makeFirstResponder(Some(&scroll));
    }
    Ok(())
}

/// Text assistance off for this box's editor. Reads no text. `roman_only` is for the visible
/// retype box: a secure field never starts an input method's composition, so keep that one the
/// same and letters arrive as typed.
fn quiet_editor(field: &NSTextField, roman_only: bool) {
    // SAFETY: currentEditor is an optional NSTextView belonging to this live field. Only
    // documented NSTextView assistance switches are sent.
    unsafe {
        let editor: Option<Retained<AnyObject>> = msg_send![field, currentEditor];
        let Some(editor) = editor else {
            return;
        };
        let _: () = msg_send![&editor, setAllowsUndo: false];
        let _: () = msg_send![&editor, setAutomaticSpellingCorrectionEnabled: false];
        let _: () = msg_send![&editor, setAutomaticTextReplacementEnabled: false];
        let _: () = msg_send![&editor, setAutomaticQuoteSubstitutionEnabled: false];
        let _: () = msg_send![&editor, setAutomaticDashSubstitutionEnabled: false];
        let _: () = msg_send![&editor, setAutomaticTextCompletionEnabled: false];
        let _: () = msg_send![&editor, setContinuousSpellCheckingEnabled: false];
        let _: () = msg_send![&editor, setGrammarCheckingEnabled: false];
        if roman_only {
            let context: Option<Retained<AnyObject>> = msg_send![&editor, inputContext];
            if let Some(context) = context {
                let roman = NSArray::from_retained_slice(&[NSString::from_str(
                    "NSAllRomanInputSourcesLocaleIdentifier",
                )]);
                let _: () = msg_send![&context, setAllowedInputSourceLocales: &*roman];
            }
        }
    }
}

/// What this label needs at this width, measured by its own cell. Set its font first.
fn fitted_height(label: &NSTextField, width: f64) -> f64 {
    // SAFETY: cell is this live label's NSCell, and cellSizeForBounds: only measures.
    let size: NSSize = unsafe {
        let cell: Retained<AnyObject> = msg_send![label, cell];
        msg_send![&cell, cellSizeForBounds: rect(0.0, 0.0, width, 10_000.0)]
    };
    size.height.ceil()
}

/// Two lines. The Recovery Kit's longest warning can need a third, and must not be cut off.
fn time_row_height(prompt: &Request, row: &NSTextField, width: f64) -> f64 {
    let longest = prompt
        .time_row_text(&TimeLeft::Left {
            seconds: 60,
            limit: TimeLimit::AutoLock,
        })
        .unwrap_or_default();
    row.setStringValue(&NSString::from_str(&longest));
    let height = fitted_height(row, width).max(40.0);
    row.setStringValue(ns_string!(""));
    height
}

fn new_hold_button(
    view: &NSView,
    caption: &str,
    frame: NSRect,
    mtm: MainThreadMarker,
) -> Retained<HoldButton> {
    // SAFETY: NSButton's documented initializer on a fresh allocation of this subclass.
    let button: Retained<HoldButton> =
        unsafe { msg_send![HoldButton::alloc(mtm), initWithFrame: frame] };
    anchor(&button);
    button.setTitle(&NSString::from_str(caption));
    button.setBezelStyle(NSBezelStyle::Push);
    view.addSubview(&button);
    button
}

fn key_button(
    view: &NSView,
    caption: &str,
    frame: NSRect,
    target: &Target,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<KeyButton> {
    // SAFETY: NSButton's documented initializer on a fresh allocation of this subclass. Target
    // implements the given selector, is retained by Panel, and outlives the button.
    let button: Retained<KeyButton> = unsafe {
        let button: Retained<KeyButton> = msg_send![KeyButton::alloc(mtm), initWithFrame: frame];
        button.setTarget(Some(target));
        button.setAction(Some(action));
        button
    };
    anchor(&button);
    button.setTitle(&NSString::from_str(caption));
    view.addSubview(&button);
    button
}

/// A line for what the owner typed, off the accessibility tree: a secure box's own value stays
/// empty there, and this must not be the way round that. Public only for the test target
/// `tests/mac_typed_line.rs`, because AppKit views need the main thread and libtest never runs a
/// test on it.
pub fn typed_line(view: &NSView, frame: NSRect, mtm: MainThreadMarker) -> Retained<NSTextField> {
    let line = label(
        view,
        "",
        frame.origin.x,
        frame.origin.y,
        frame.size.width,
        frame.size.height,
        mtm,
    );
    line.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(16.0, 0.0)));
    hide_from_accessibility(&line);
    line
}

fn hide_from_accessibility(label: &NSTextField) {
    // AppKit exposes a label through its cell, so the label's own flag isn't enough.
    // SAFETY: The documented NSAccessibility setter on this live label and its live cell.
    unsafe {
        let _: () = msg_send![label, setAccessibilityElement: false];
        let cell: Retained<AnyObject> = msg_send![label, cell];
        let _: () = msg_send![&cell, setAccessibilityElement: false];
    }
}

/// Every write to the feedback row. Resetting the color each time keeps Please wait… from red.
fn set_feedback(panel: &Panel, text: &str, is_error: bool) {
    panel.feedback.setStringValue(&NSString::from_str(text));
    let color = if is_error {
        NSColor::systemRedColor()
    } else {
        NSColor::labelColor()
    };
    panel.feedback.setTextColor(Some(&color));
}

/// The row under a Recovery Key box: a count only, then the usual hint.
fn recovery_feedback(prompt: &Request, value: &str) -> String {
    format!("{}. {}", recovery_progress(value), prompt.feedback_text())
}

/// The feedback row when no refusal or question is waiting to be read.
fn normal_feedback(panel: &Panel) -> String {
    if panel.suggestion.is_some() {
        return SUGGESTED_TEXT.to_owned();
    }
    if let Some(line) = &panel.live_line {
        return line.clone();
    }
    if panel.prompt.offers_show_typing()
        && let Some(field) = panel.fields.first()
    {
        let value = Zeroizing::new(field.stringValue().to_string());
        return recovery_feedback(&panel.prompt, &value);
    }
    panel.prompt.feedback_text()
}

fn restore_feedback(panel: &mut Panel) {
    panel.error_shown = panel.last_error.is_some();
    match &panel.last_error {
        Some(message) => set_feedback(panel, &problem_text(message), true),
        None => set_feedback(panel, &normal_feedback(panel), false),
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

/// AppKit coordinates point upward. Consume rows downward without overlapping their frames.
fn place(y: &mut f64, height: f64, gap: f64) -> f64 {
    *y -= height;
    let bottom = *y;
    *y -= gap;
    bottom
}

/// The NSEventType AppKit is handling, and whether it's an auto-repeated key.
fn current_event() -> Option<(usize, bool)> {
    // SAFETY: Called on AppKit's main thread. currentEvent is an optional live NSEvent, and
    // isARepeat is queried only for a key-down event.
    unsafe {
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        let event: Option<Retained<AnyObject>> = msg_send![&app, currentEvent];
        let event = event?;
        let kind: usize = msg_send![&event, type];
        Some((kind, kind == 10 && msg_send![&event, isARepeat]))
    }
}

fn current_event_repeats() -> bool {
    current_event().is_some_and(|(_, repeated)| repeated)
}

fn activity(genuine: bool) -> bool {
    // A repeated key revalidates without keeping Auto-lock alive.
    let genuine = genuine && !current_event_repeats();
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
    let restored = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut().filter(|panel| panel.id == id)?;
        panel.activity = Some(hook);
        Some(())
    });
    restored.is_some() && apply_authorization(id, authorization)
}

fn apply_authorization(id: u64, authorization: Authorization) -> bool {
    let now = Instant::now();
    let update = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut().filter(|panel| panel.id == id)?;
        let grace = panel.prompt.rides_out_a_blip();
        let update = panel.presentation.authorize(authorization, now, grace);
        match update {
            PresentationUpdate::Clear => {
                for label in &panel.sensitive_labels {
                    label.setStringValue(ns_string!(""));
                }
                if let (Some(readout), Some(scroll)) = (&panel.readout, &panel.readout_scroll) {
                    set_readout(readout, scroll, "");
                }
                panel.readout_shown = false;
                // The key is hidden for now, so its visible retyped part must be too. Alpha, not
                // hidden: a hidden box would lose focus, and the owner may be typing.
                if panel.prompt.shows_typing() {
                    for field in &panel.fields {
                        field.setAlphaValue(0.0);
                    }
                }
                hide_typed(panel);
                panel.cancel_arm.disarm();
                if let Some(suggestion) = panel.suggestion.as_mut() {
                    suggestion.acknowledged = false;
                }
                set_feedback(panel, "Please wait…", false);
            }
            PresentationUpdate::Restore => {
                if let Request::Entry(Prompt::RecoveryKit { recovery, .. }) = &panel.prompt
                    && let Some(label) = panel.sensitive_labels.first()
                {
                    label.setStringValue(&NSString::from_str(recovery));
                    for field in &panel.fields {
                        field.setAlphaValue(1.0);
                    }
                }
                if let (Request::Reveal { reveal, .. }, Some(readout), Some(scroll)) =
                    (&panel.prompt, &panel.readout, &panel.readout_scroll)
                {
                    panel.readout_shown = reveal.mode == RevealMode::Timed
                        && panel
                            .expires
                            .is_some_and(|expires| Instant::now() < expires);
                    set_readout(
                        readout,
                        scroll,
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
                    label.setStringValue(&NSString::from_str(&suggestion.text));
                }
                // Restore only a generated candidate. What the owner typed stays off screen: both
                // mirrors stay blank until asked for again.
                restore_feedback(panel);
            }
            _ => (),
        }
        Some(update)
    });
    if update == Some(PresentationUpdate::Close) {
        finish(Err(PromptError::Interrupted));
    }
    matches!(
        update,
        Some(PresentationUpdate::Keep | PresentationUpdate::Restore)
    )
}

/// Both opt-in mirrors off and blank. Neither comes back by itself.
fn hide_typed(panel: &mut Panel) {
    if let Some(checkbox) = &panel.show_typing {
        // SAFETY: NSButton's documented state setter; 0 is NSControlStateValueOff. Sends no action.
        unsafe {
            let _: () = msg_send![checkbox, setState: 0isize];
        }
        if let Some(mirror) = panel.sensitive_labels.first() {
            mirror.setStringValue(ns_string!(""));
        }
    }
    panel.typed_key_at = None;
    end_hold(panel);
}

fn end_hold(panel: &mut Panel) {
    panel.hold_until = None;
    if let Some(mirror) = &panel.hold_mirror {
        mirror.setStringValue(ns_string!(""));
    }
    if let Some(caption) = &panel.hold_caption {
        caption.setStringValue(ns_string!(""));
    }
    if let Some(button) = &panel.typed_hold {
        button.highlight(false);
    }
}

/// Nothing typed shows while a known sharing app runs. A busy core changes nothing here: it says
/// nothing about sharing apps, and a long one ends in `Clear`. Holds no ACTIVE borrow while it
/// disables controls: that can end a control's focus, which calls back.
fn update_show_controls() {
    let controls = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        if panel.show_typing.is_none() && panel.typed_hold.is_none() {
            return None;
        }
        let check = (panel.may_show)();
        let (usable, blank) = check.controls(panel.show_usable);
        panel.show_usable = usable;
        if blank {
            hide_typed(panel);
        }
        Some((
            usable,
            // A press with nothing to show would do nothing and say nothing.
            usable && !panel.boxes_empty && panel.suggestion.is_none(),
            (check != ShowCheck::Busy)
                .then(|| panel.show_refused.clone())
                .flatten()
                .map(|line| {
                    (
                        line,
                        check == ShowCheck::Refused && panel.suggestion.is_none(),
                    )
                }),
            panel.show_typing.clone(),
            panel.typed_hold.clone(),
        ))
    });
    let Some((usable, hold_usable, refused, show_typing, typed_hold)) = controls else {
        return;
    };
    if let Some(checkbox) = &show_typing {
        checkbox.setEnabled(usable);
    }
    if let Some(button) = &typed_hold {
        button.setEnabled(hold_usable);
    }
    if let Some((line, show)) = &refused {
        line.setHidden(!show);
    }
}

fn show_typing_changed() {
    let passed = activity(true);
    let now = Instant::now();
    ACTIVE.with(|active| {
        let Ok(mut active) = active.try_borrow_mut() else {
            return;
        };
        let Some(panel) = active.as_mut() else {
            return;
        };
        // A click never shows something on a guess: anything but a clear yes unticks it.
        if !passed || (panel.may_show)() != ShowCheck::Allowed {
            hide_typed(panel);
            return;
        }
        let ticked = panel.show_typing.as_ref().is_some_and(|checkbox| {
            // SAFETY: NSButton's documented state getter; 1 is NSControlStateValueOn.
            let state: isize = unsafe { msg_send![&**checkbox, state] };
            state == 1
        });
        panel.typed_key_at = ticked.then_some(now);
    });
    if passed {
        refresh_recovery_lines();
    }
}

/// The count and, when ticked, the mirror. Call only after activity() returned true.
fn refresh_recovery_lines() {
    ACTIVE.with(|active| {
        let Ok(active) = active.try_borrow() else {
            return;
        };
        let Some(panel) = active.as_ref() else {
            return;
        };
        let (true, Some(field)) = (panel.prompt.offers_show_typing(), panel.fields.first()) else {
            return;
        };
        let value = Zeroizing::new(field.stringValue().to_string());
        if !panel.presentation.suspended && !panel.error_shown {
            set_feedback(panel, &recovery_feedback(&panel.prompt, &value), false);
        }
        if let (Some(checkbox), Some(mirror)) = (&panel.show_typing, panel.sensitive_labels.first())
        {
            // SAFETY: NSButton's documented state getter; 1 is NSControlStateValueOn.
            let state: isize = unsafe { msg_send![checkbox, state] };
            let shown = if state == 1 {
                recovery_mirror(&value)
            } else {
                Zeroizing::new(String::new())
            };
            mirror.setStringValue(&NSString::from_str(&shown));
        }
    });
}

fn edit_activity_is_genuine() -> bool {
    // Only input events can renew an edit lease; programmatic notifications revalidate.
    current_event().is_some_and(|(kind, repeated)| edit_event_is_genuine(Some(kind), repeated))
}

fn edit_event_is_genuine(kind: Option<usize>, repeated: bool) -> bool {
    // NSEventType: keyDown, or mouse buttons used by native context-menu Paste.
    matches!(kind, Some(1 | 2 | 3 | 4 | 25 | 26)) || kind == Some(10) && !repeated
}

fn anchor(view: &NSView) {
    // Keep a row centered in its container, which itself stays centered when the main
    // protected window changes size. Fixed widths fit its configured minimum size.
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewMinXMargin
            | NSAutoresizingMaskOptions::ViewMaxXMargin
            | NSAutoresizingMaskOptions::ViewMinYMargin,
    );
}

fn label(
    view: &NSView,
    text: &str,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    mtm: MainThreadMarker,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
    label.setFrame(rect(x, y, width, height));
    anchor(&label);
    label.setSelectable(false);
    // SAFETY: These are documented NSTextField/NSCell layout switches. Readouts can contain
    // a complete Seed Phrase or generated passphrase and must wrap rather than truncate.
    unsafe {
        let _: () = msg_send![&label, setMaximumNumberOfLines: 0isize];
        let cell: Retained<AnyObject> = msg_send![&label, cell];
        let _: () = msg_send![&cell, setWraps: true];
        let _: () = msg_send![&cell, setScrollable: false];
        let _: () = msg_send![&label, unregisterDraggedTypes];
    }
    view.addSubview(&label);
    label
}

fn button(
    view: &NSView,
    caption: &str,
    frame: NSRect,
    target: &Target,
    action: Sel,
    mtm: MainThreadMarker,
) -> Retained<NSButton> {
    let button = NSButton::initWithFrame(NSButton::alloc(mtm), frame);
    anchor(&button);
    button.setTitle(&NSString::from_str(caption));
    button.setBezelStyle(NSBezelStyle::Push);
    // SAFETY: Target implements the given selector, is retained by Panel, and outlives the button.
    unsafe {
        button.setTarget(Some(target));
        button.setAction(Some(action));
        view.addSubview(&button);
    }
    button
}

fn same_object<A, B>(a: &A, b: &B) -> bool {
    std::ptr::eq(
        std::ptr::from_ref(a).cast::<()>(),
        std::ptr::from_ref(b).cast::<()>(),
    )
}

/// Only a positive match counts: anything unknown is treated as New or Confirm.
fn is_current_box(prompt: &Request, index: Option<usize>) -> bool {
    index == Some(0)
        && matches!(
            prompt,
            Request::Entry(Prompt::NewPassword {
                current_password: true,
                ..
            })
        )
}

/// Which boxes to clear, and which to focus, after a refusal. `count` is how many boxes there are.
fn refusal_targets(count: usize, focus: RefusalFocus) -> Option<(Range<usize>, usize)> {
    match focus {
        RefusalFocus::Keep => None,
        RefusalFocus::ClearAndFocusNew => {
            let new = count.checked_sub(2)?;
            Some((new..count, new))
        }
        RefusalFocus::FocusConfirm => {
            let confirm = count.checked_sub(1)?;
            Some((confirm..confirm, confirm))
        }
        RefusalFocus::ClearAndFocusRetype => {
            count.checked_sub(1)?;
            Some((0..count, 0))
        }
    }
}

/// Clears through the field editor and then the cell, like `finish`. AppKit can call back from
/// these calls, so the caller holds no ACTIVE borrow. Setting a cell can end editing, so the
/// caller also puts the focus back.
fn clear_boxes(fields: &[Retained<NSTextField>]) {
    SETTING_BOXES.set(true);
    for field in fields {
        // SAFETY: currentEditor is the optional live NSTextView for this field; setString clears
        // its current input. AppKit may retain OS-owned copies that cannot be wiped by Rust.
        unsafe {
            let editor: Option<Retained<AnyObject>> = msg_send![field, currentEditor];
            if let Some(editor) = editor {
                let _: () = msg_send![&editor, setString: ns_string!("")];
            }
        }
        field.setStringValue(ns_string!(""));
    }
    SETTING_BOXES.set(false);
}

fn submit() {
    if !activity(false) {
        return;
    }
    if ACTIVE.with(|active| {
        active.try_borrow().ok().is_some_and(|active| {
            active
                .as_ref()
                .is_some_and(|panel| matches!(&panel.prompt, Request::Reveal { .. }))
        })
    }) {
        finish(Err(PromptError::Cancelled));
        return;
    }
    let outcome = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        panel.cancel_arm.disarm();
        let values = panel
            .fields
            .iter()
            .map(|field| Zeroizing::new(field.stringValue().to_string()))
            .collect();
        match panel.prompt.reply(values) {
            Ok(reply) => {
                if let Some(suggestion) = panel.suggestion.as_mut()
                    && !suggestion.acknowledged
                {
                    suggestion.acknowledged = true;
                    set_feedback(panel, SUGGESTION_ASK, false);
                    return None;
                }
                Some(Ok(reply))
            }
            Err(refusal) => {
                set_feedback(panel, &problem_text(&refusal.message), true);
                panel.error_shown = true;
                let targets = refusal_targets(panel.fields.len(), refusal.focus);
                panel.last_error = Some(refusal.message.into_owned());
                let Some((clear, focus)) = targets else {
                    return Some(Err(None));
                };
                if !clear.is_empty() {
                    panel.live_line = None;
                    panel.last_edited = None;
                    panel.boxes_empty = true;
                }
                Some(Err(Some((
                    panel.window.clone(),
                    panel.fields[clear].to_vec(),
                    panel.fields[focus].clone(),
                ))))
            }
        }
    });
    match outcome {
        Some(Ok(reply)) => finish(Ok(reply)),
        Some(Err(Some((window, clear, focus)))) => {
            clear_boxes(&clear);
            window.makeFirstResponder(Some(&focus));
        }
        Some(Err(None)) => focus_kept_refusal(),
        None => {}
    }
}

/// After a refusal that keeps the boxes. A button with the keyboard would eat the correction typed
/// next, but a caret already in a box stays where it is. Checks only the lengths.
fn focus_kept_refusal() {
    let target = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        let mut empty = Vec::with_capacity(panel.fields.len());
        for field in &panel.fields {
            // SAFETY: currentEditor is this live field's optional field editor. Nothing is read.
            let editor: Option<Retained<AnyObject>> = unsafe { msg_send![&**field, currentEditor] };
            if editor.is_some() {
                return None;
            }
            empty.push(field.stringValue().length() == 0);
        }
        let field = panel.fields.get(kept_refusal_box(&empty)?)?;
        Some((panel.window.clone(), field.clone()))
    });
    if let Some((window, field)) = target {
        focus_at_end(&window, &field);
    }
}

/// Focusing selects the whole box, and the next key would replace it. AppKit can call back from
/// here, so the caller holds no ACTIVE borrow.
fn focus_at_end(window: &NSWindow, field: &NSTextField) {
    window.makeFirstResponder(Some(field));
    // SAFETY: currentEditor is the optional live NSTextView for this field, and this is
    // NSResponder's documented caret action. Nothing is read.
    unsafe {
        let editor: Option<Retained<AnyObject>> = msg_send![field, currentEditor];
        if let Some(editor) = editor {
            let _: () = msg_send![&editor, moveToEndOfDocument: Option::<&AnyObject>::None];
        }
    }
}

fn return_pressed() {
    if current_event_repeats() || !activity(true) {
        return;
    }
    // Length only: nothing here copies what was typed.
    let empty = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        if !matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. })) {
            return None;
        }
        let field = panel
            .fields
            .iter()
            .find(|field| field.stringValue().length() == 0)?;
        Some((panel.window.clone(), field.clone()))
    });
    match empty {
        Some((window, field)) => {
            window.makeFirstResponder(Some(&field));
        }
        None => submit(),
    }
}

/// The owner's Cancel or Escape. On the Recovery Kit the first press only asks, because this key
/// is shown only now. Losing focus, a denied lease and `cancel(id)` stay immediate.
fn owner_cancel() {
    let is_kit = ACTIVE.with(|active| {
        active
            .try_borrow()
            .ok()
            .is_some_and(|active| active.as_ref().is_some_and(|panel| panel.prompt.is_kit()))
    });
    if !is_kit {
        activity(true);
        finish(Err(PromptError::Cancelled));
        return;
    }
    if current_event_repeats() {
        return;
    }
    // Denied has already closed the panel. A blanked Kit rejects this press like any other action.
    if !activity(true) && !kit_on_hold() {
        return;
    }
    let from_key = current_event().is_some_and(|(kind, _)| kind == 10);
    let decision = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        let decision = panel.cancel_arm.press(Instant::now(), from_key);
        if decision == ArmDecision::Arm {
            set_feedback(panel, KIT_ARMED, false);
        }
        Some(decision)
    });
    if decision == Some(ArmDecision::Cancel) {
        finish(Err(PromptError::Cancelled));
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

fn text_changed(notification: &AnyObject) {
    if SETTING_BOXES.get() {
        return;
    }
    // SAFETY: A control's text notification has that control as its documented object.
    let edited: Option<Retained<AnyObject>> = unsafe { msg_send![notification, object] };
    let genuine = edit_activity_is_genuine();
    let now = Instant::now();
    let forgotten = ACTIVE.with(|active| {
        // Never borrow_mut here: AppKit can send this from inside another call.
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        let armed = panel.cancel_arm.armed_at.is_some();
        if !panel.presentation.suspended && (panel.error_shown || armed) {
            panel.error_shown = false;
            panel.last_error = None;
            set_feedback(panel, &normal_feedback(panel), false);
        }
        panel.cancel_arm.disarm();
        if genuine && panel.typed_key_at.is_some() {
            panel.typed_key_at = Some(now);
        }
        let index = edited.as_deref().and_then(|edited| {
            panel
                .fields
                .iter()
                .position(|field| same_object(&**field, edited))
        });
        if !matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. }))
            || is_current_box(&panel.prompt, index)
        {
            return None;
        }
        if panel.suggestion.take().is_none() {
            panel.last_edited = index;
            panel.boxes_empty = false;
            // Every edit, repeats too: `genuine` is only about keeping Auto-lock alive.
            panel.live.box_edited(now, panel.fields.len(), index);
            return None;
        }
        panel.live_line = None;
        panel.last_edited = None;
        panel.boxes_empty = true;
        panel.error_shown = true;
        let new_box = panel.fields.len().checked_sub(2)?;
        Some((
            panel.window.clone(),
            panel.fields[new_box..].to_vec(),
            index.and_then(|index| panel.fields.get(index)).cloned(),
        ))
    });
    if let Some((window, boxes, edited)) = forgotten {
        clear_boxes(&boxes);
        if let Some(edited) = &edited {
            window.makeFirstResponder(Some(edited));
        }
        ACTIVE.with(|active| {
            if let Ok(active) = active.try_borrow()
                && let Some(panel) = active.as_ref()
            {
                if let Some(label) = panel.sensitive_labels.last() {
                    label.setStringValue(ns_string!(""));
                }
                if !panel.presentation.suspended {
                    set_feedback(panel, SUGGESTION_REMOVED, false);
                }
            }
        });
    }
    if activity(genuine) {
        refresh_recovery_lines();
    } else {
        ACTIVE.with(|active| {
            if let Ok(mut active) = active.try_borrow_mut()
                && let Some(panel) = active.as_mut()
            {
                panel.lines_stale = true;
            }
        });
    }
}

/// The box a hold shows: the New or Confirm box with the focus and some text, otherwise the one
/// typed in last. `new_box` is where New sits, and nothing before it is ever shown.
fn hold_target(
    new_box: usize,
    focused_with_text: Option<usize>,
    last_edited: Option<usize>,
) -> Option<usize> {
    focused_with_text
        .filter(|focused| *focused >= new_box)
        .or(last_edited)
}

/// A snapshot at press, with no updates while held. A press never shows something on a guess, so
/// only a clear yes counts here.
fn hold_typed(panel: &mut Panel) {
    if panel.hold_until.is_some()
        || panel.suggestion.is_some()
        || (panel.may_show)() != ShowCheck::Allowed
    {
        return;
    }
    let new_box = panel.fields.len().saturating_sub(2);
    let focused = panel
        .fields
        .iter()
        .enumerate()
        .skip(new_box)
        .find(|(_, field)| {
            // SAFETY: currentEditor is this live field's optional field editor. Nothing is read.
            let editor: Option<Retained<AnyObject>> =
                unsafe { msg_send![&***field, currentEditor] };
            editor.is_some() && field.stringValue().length() > 0
        })
        .map(|(index, _)| index);
    let Some(index) = hold_target(new_box, focused, panel.last_edited) else {
        return;
    };
    let Some(field) = panel.fields.get(index) else {
        return;
    };
    let typed = Zeroizing::new(field.stringValue().to_string());
    if let Some(mirror) = &panel.hold_mirror {
        mirror.setStringValue(&NSString::from_str(&typed));
    }
    if let (Some(caption), Some(name)) = (&panel.hold_caption, panel.prompt.labels().get(index)) {
        caption.setStringValue(&NSString::from_str(name));
    }
    panel.hold_until = Some(Instant::now() + HOLD_LIMIT);
}

/// A keyboard hold leaves focus on the button. A mouse hold never moved it.
fn return_hold_focus(button: &HoldButton) {
    let target = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        let field = panel
            .last_edited
            .and_then(|index| panel.fields.get(index))?;
        Some((panel.window.clone(), field.clone()))
    });
    let Some((window, field)) = target else {
        return;
    };
    if !window
        .firstResponder()
        .is_some_and(|responder| same_object(&*responder, button))
    {
        return;
    }
    focus_at_end(&window, &field);
}

/// The new-password hold button isn't a readout's control: Return and Escape aren't Close here.
fn typed_hold_key(event: &AnyObject, repeated: bool) -> bool {
    let holds_typing = ACTIVE.with(|active| {
        active.try_borrow().ok().is_some_and(|active| {
            active
                .as_ref()
                .is_some_and(|panel| panel.prompt.offers_hold_to_show())
        })
    });
    if !holds_typing {
        return false;
    }
    // SAFETY: This callback receives a live NSEvent from the native hold control.
    let text: Option<Retained<NSString>> = unsafe { msg_send![event, charactersIgnoringModifiers] };
    let Some(text) = text else {
        return false;
    };
    match text.to_string().as_str() {
        "\u{1b}" | "\r" | " " if repeated => true,
        "\u{1b}" => {
            owner_cancel();
            true
        }
        "\r" => {
            return_pressed();
            true
        }
        " " => {
            held(true);
            true
        }
        _ => false,
    }
}

fn held(show: bool) {
    if show && !activity(false) {
        return;
    }
    ACTIVE.with(|active| {
        let Ok(mut active) = active.try_borrow_mut() else {
            return;
        };
        let Some(panel) = active.as_mut() else {
            return;
        };
        if panel.prompt.offers_hold_to_show() {
            if show {
                hold_typed(panel);
            } else {
                end_hold(panel);
            }
            return;
        }
        if let (Request::Reveal { reveal, .. }, Some(readout), Some(scroll)) =
            (&panel.prompt, &panel.readout, &panel.readout_scroll)
            && reveal.mode == RevealMode::Held
            && panel.readout_shown != show
        {
            panel.readout_shown = show;
            set_readout(
                readout,
                scroll,
                if show {
                    &reveal.value
                } else {
                    "Hold Space or the button to show. Scroll to read long values."
                },
            );
        }
    });
}

fn set_readout(readout: &NSView, scroll: &ReadoutScroll, value: &str) {
    let (display, lines) = readout_text(value);
    let height = ((lines as f64) * 24.0 + 12.0).max(112.0);
    readout.setFrameSize(NSSize::new(scroll.contentSize().width, height));
    // SAFETY: The view was constructed as a read-only, nonselectable NSTextView above.
    unsafe {
        let _: () = msg_send![readout, setString: &*NSString::from_str(&display)];
    }
    let clip = scroll.contentView();
    clip.scrollToPoint(NSPoint::new(0.0, 0.0));
    scroll.reflectScrolledClipView(&clip);
}

fn is_space_event(event: &AnyObject) -> bool {
    // SAFETY: Called only for AppKit-supplied native keyboard NSEvents.
    let text: Option<Retained<NSString>> = unsafe { msg_send![event, charactersIgnoringModifiers] };
    text.is_some_and(|text| text.to_string() == " ")
}

fn reveal_navigation(event: &AnyObject) -> bool {
    // SAFETY: This callback receives a live NSEvent from a native readout/hold control.
    let text: Option<Retained<NSString>> = unsafe { msg_send![event, charactersIgnoringModifiers] };
    let Some(text) = text else {
        return false;
    };
    match text.to_string().as_str() {
        "\u{1b}" | "\r" => {
            finish(Err(PromptError::Cancelled));
            true
        }
        " " => {
            let is_held = ACTIVE.with(|active| active.try_borrow().ok().is_some_and(|active| active.as_ref().is_some_and(|panel| matches!(&panel.prompt, Request::Reveal { reveal, .. } if reveal.mode == RevealMode::Held))));
            if is_held {
                held(true);
            }
            is_held
        }
        "\t" | "\u{19}" => {
            let window = ACTIVE.with(|active| {
                let active = active.try_borrow().ok()?;
                active.as_ref().map(|panel| panel.window.clone())
            });
            if let Some(window) = window {
                // SAFETY: The retained main window owns the complete native key-view loop.
                unsafe {
                    if text.to_string() == "\u{19}" {
                        let _: () =
                            msg_send![&window, selectPreviousKeyView: Option::<&AnyObject>::None];
                    } else {
                        let _: () =
                            msg_send![&window, selectNextKeyView: Option::<&AnyObject>::None];
                    }
                }
            }
            true
        }
        key @ ("\u{f700}" | "\u{f701}" | "\u{f72c}" | "\u{f72d}" | "\u{f729}" | "\u{f72b}") => {
            ACTIVE.with(|active| {
                let Ok(active) = active.try_borrow() else {
                    return;
                };
                let Some(panel) = active.as_ref() else {
                    return;
                };
                let (Some(scroll), Some(readout)) = (&panel.readout_scroll, &panel.readout) else {
                    return;
                };
                let clip = scroll.contentView();
                let bounds = clip.bounds();
                let top = (readout.frame().size.height - bounds.size.height).max(0.0);
                let y = match key {
                    "\u{f700}" => bounds.origin.y - 24.0,
                    "\u{f701}" => bounds.origin.y + 24.0,
                    "\u{f72c}" => bounds.origin.y - bounds.size.height,
                    "\u{f72d}" => bounds.origin.y + bounds.size.height,
                    "\u{f729}" => 0.0,
                    _ => top,
                }
                .clamp(0.0, top);
                clip.scrollToPoint(NSPoint::new(0.0, y));
                scroll.reflectScrolledClipView(&clip);
            });
            true
        }
        _ => false,
    }
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
        if present && !apply_authorization(id, authorization) {
            return;
        }
    }
    tick();
}

fn caps_lock_on() -> bool {
    // SAFETY: NSEvent's documented class property; bit 16 is NSEventModifierFlagCapsLock.
    let flags: usize = unsafe { msg_send![class!(NSEvent), modifierFlags] };
    flags & (1 << 16) != 0
}

/// Capitals change a password. They don't change a Recovery Key.
fn caps_lock_matters(prompt: &Request) -> bool {
    matches!(
        prompt,
        Request::Entry(
            Prompt::Credential {
                recovery: false,
                ..
            } | Prompt::NewPassword { .. }
        )
    )
}

fn time_row_line(caps_lock: bool, time: &str) -> String {
    match (caps_lock, time.is_empty()) {
        (false, _) => time.to_owned(),
        (true, true) => CAPS_LOCK_TEXT.to_owned(),
        (true, false) => format!("{CAPS_LOCK_TEXT} {time}"),
    }
}

/// The probe is read only: asking never counts as activity.
fn update_time_row(panel: &mut Panel) {
    // None is a busy core: keep the last answer.
    if let Some(text) = panel.prompt.time_row_text(&(panel.time_left)()) {
        panel.time_text = text;
    }
    let caps_lock = caps_lock_matters(&panel.prompt) && caps_lock_on();
    let line = time_row_line(caps_lock, &panel.time_text);
    if line != panel.time_row_shown {
        panel.time_row.setStringValue(&NSString::from_str(&line));
        panel.time_row_shown = line;
    }
}

/// At typing pauses, from New and Confirm only. Nothing here approves anything: Continue always
/// runs the real check.
fn update_live_line(panel: &mut Panel, now: Instant) {
    if !matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. }))
        || panel.suggestion.is_some()
        || panel.presentation.suspended
        || panel.error_shown
    {
        return;
    }
    if !panel.live.poll() && !panel.live.due(now) {
        return;
    }
    let [.., new, confirm] = panel.fields.as_slice() else {
        return;
    };
    let new = Zeroizing::new(new.stringValue().to_string());
    let confirm = Zeroizing::new(confirm.stringValue().to_string());
    panel.boxes_empty = new.is_empty() && confirm.is_empty();
    let line = panel.live.line(&new, &confirm);
    set_feedback(panel, &line, false);
    panel.live_line = Some(line);
}

/// True once Show what I type has had no real key press for too long.
fn typed_view_expired(typed_key_at: Option<Instant>, now: Instant) -> bool {
    typed_key_at.is_some_and(|at| now.saturating_duration_since(at) >= TYPED_IDLE_LIMIT)
}

fn tick() {
    if !activity(false) {
        return;
    }
    let now = Instant::now();
    let stale = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut()?;
        if panel.cancel_arm.expired(now) {
            panel.cancel_arm.disarm();
            restore_feedback(panel);
        }
        update_time_row(panel);
        if panel.hold_until.is_some_and(|until| now >= until) {
            end_hold(panel);
        }
        if typed_view_expired(panel.typed_key_at, now) {
            hide_typed(panel);
        }
        update_live_line(panel, now);
        Some(std::mem::take(&mut panel.lines_stale))
    });
    if stale == Some(true) {
        refresh_recovery_lines();
    }
    update_show_controls();
    let expired = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        let expires = panel.expires?;
        let Request::Reveal { reveal, .. } = &panel.prompt else {
            return None;
        };
        let remaining = expires.min(reveal.closes_at).saturating_duration_since(now);
        let text = timed_readout_text(remaining.as_secs() + 1, 3 - panel.extensions);
        set_feedback(panel, &text, false);
        Some(remaining.is_zero())
    });
    if expired == Some(true) {
        finish(Err(PromptError::Cancelled));
    }
}

fn suggest(sender: Option<&AnyObject>) {
    if !activity(false) {
        return;
    }
    let boxes = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        if !matches!(&panel.prompt, Request::Entry(Prompt::NewPassword { .. })) {
            return None;
        }
        let new_box = panel.fields.len().checked_sub(2)?;
        Some(panel.fields[new_box..].to_vec())
    });
    let Some(boxes) = boxes else {
        return;
    };
    let suggestion = policy::suggest_passphrase();
    let string = NSString::from_str(&suggestion);
    // AppKit can call back from these, so no ACTIVE borrow is held.
    SETTING_BOXES.set(true);
    for field in &boxes {
        field.setStringValue(&string);
    }
    SETTING_BOXES.set(false);
    ACTIVE.with(|active| {
        let Ok(mut active) = active.try_borrow_mut() else {
            return;
        };
        let Some(panel) = active.as_mut() else {
            return;
        };
        end_hold(panel);
        if let Some(label) = panel.sensitive_labels.last() {
            label.setStringValue(&string);
        }
        panel.suggestion = Some(Suggestion {
            text: suggestion,
            acknowledged: false,
        });
        panel.error_shown = false;
        panel.last_error = None;
        panel.live_line = None;
        panel.last_edited = None;
        panel.boxes_empty = false;
        set_feedback(panel, SUGGESTED_TEXT, false);
    });
    // Off the button, or the next Space replaces a password the owner may have just written down.
    // An empty Current box comes first: typing there leaves the suggestion alone. A click never
    // gave the button the keyboard, so it leaves the focus where it was.
    let target = ACTIVE.with(|active| {
        let active = active.try_borrow().ok()?;
        let panel = active.as_ref()?;
        let (responder, sender) = panel.window.firstResponder().zip(sender)?;
        if !same_object(&*responder, sender) {
            return None;
        }
        let field = panel
            .fields
            .iter()
            .find(|field| field.stringValue().length() == 0)
            .or_else(|| panel.fields.get(panel.fields.len().checked_sub(2)?))?;
        Some((panel.window.clone(), field.clone()))
    });
    if let Some((window, field)) = target {
        window.makeFirstResponder(Some(&field));
    }
}

fn finish(result: Result<Reply, PromptError>) {
    let panel = ACTIVE.with(|active| active.borrow_mut().take());
    let Some(panel) = panel else {
        return;
    };
    // SAFETY: Clear run-loop callbacks before removing/releasing their retained target.
    unsafe {
        if let Some(timer) = &panel.timer {
            let _: () = msg_send![timer, invalidate];
        }
        let center: Retained<AnyObject> = msg_send![class!(NSNotificationCenter), defaultCenter];
        let _: () = msg_send![&center, removeObserver: &*panel._target];
    }
    // Clear both the shared native field editor and each cell before removing any controls.
    for field in &panel.fields {
        // SAFETY: currentEditor is the optional live NSTextView for this field; setString clears
        // its current input. AppKit may retain OS-owned copies that cannot be wiped by Rust.
        unsafe {
            let editor: Option<Retained<AnyObject>> = msg_send![field, currentEditor];
            if let Some(editor) = editor {
                // Don't leave the keyboard on ABC.
                if panel.prompt.shows_typing() {
                    let context: Option<Retained<AnyObject>> = msg_send![&editor, inputContext];
                    if let Some(context) = context {
                        let _: () = msg_send![&context, setAllowedInputSourceLocales: Option::<&AnyObject>::None];
                    }
                }
                let _: () = msg_send![&editor, setString: ns_string!("")];
            }
            let _: () = msg_send![field, setDelegate: Option::<&AnyObject>::None];
        }
        field.setStringValue(ns_string!(""));
    }
    for label in &panel.sensitive_labels {
        label.setStringValue(ns_string!(""));
    }
    if let Some(mirror) = &panel.hold_mirror {
        mirror.setStringValue(ns_string!(""));
    }
    if let Some(readout) = &panel.readout {
        // SAFETY: Clear the native NSTextView storage, including its accessibility value,
        // before the view is removed. Undo and selection were never enabled.
        unsafe {
            let _: () = msg_send![readout, setString: ns_string!("")];
        }
    }
    panel.window.makeFirstResponder(None);
    panel.window.setContentView(Some(&panel.original));
    if let Some(responder) = &panel.original_focus {
        panel.window.makeFirstResponder(Some(responder));
    }
    if let Some(editor) = RETYPE_EDITOR.with(|editor| editor.borrow_mut().take()) {
        // SAFETY: A live NSTextView that no box is editing with any more.
        unsafe {
            let _: () = msg_send![&editor, setString: ns_string!("")];
        }
    }
    panel.completion.finish(result);
}

pub(super) fn cancel(id: u64) -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    let matches =
        ACTIVE.with(|active| active.borrow().as_ref().is_some_and(|panel| panel.id == id));
    if matches {
        finish(Err(PromptError::Cancelled));
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::{
        HoldButton, KeyButton, RetypeCell, RetypeEditor, RetypeField, TYPED_IDLE_LIMIT,
        caps_lock_matters, edit_event_is_genuine, hold_target, is_current_box, refusal_targets,
        time_row_line, typed_view_expired,
    };
    use crate::native_credential::{Prompt, RefusalFocus, Request};
    use objc2::runtime::{AnyClass, Sel};
    use objc2::{ClassType, sel};
    use objc2_app_kit::NSTextView;
    use std::time::{Duration, Instant};

    fn credential(recovery: bool) -> Request {
        Request::Entry(Prompt::Credential {
            title: "Fake action".into(),
            detail: String::new(),
            label: "Fake label".into(),
            recovery,
        })
    }

    fn new_password(current_password: bool) -> Request {
        Request::Entry(Prompt::NewPassword {
            title: "Fake setup".into(),
            detail: String::new(),
            current_password,
        })
    }

    #[test]
    fn credential_edits_count_input_but_not_repeats_or_programmatic_notifications() {
        assert!(edit_event_is_genuine(Some(10), false));
        for event in [1, 2, 3, 4, 25, 26] {
            assert!(edit_event_is_genuine(Some(event), false));
        }
        assert!(!edit_event_is_genuine(Some(10), true));
        for event in [None, Some(11), Some(13), Some(15)] {
            assert!(!edit_event_is_genuine(event, false));
        }
    }

    #[test]
    fn only_a_positive_match_is_the_current_box() {
        let change = new_password(true);
        assert!(is_current_box(&change, Some(0)));
        assert!(!is_current_box(&change, Some(1)));
        assert!(!is_current_box(&change, Some(2)));
        assert!(!is_current_box(&change, None));
        // Setup has no Current box: its first box is New.
        assert!(!is_current_box(&new_password(false), Some(0)));
        assert!(!is_current_box(&credential(false), Some(0)));
    }

    #[test]
    fn a_hold_shows_the_focused_box_or_the_last_edited_and_never_current() {
        // Change password: Current, New, Confirm. Both typed, New clicked again.
        assert_eq!(hold_target(1, Some(1), Some(2)), Some(1));
        assert_eq!(hold_target(1, Some(2), Some(1)), Some(2));
        // Focus elsewhere, or in an empty box.
        assert_eq!(hold_target(1, None, Some(2)), Some(2));
        // Focus in Current never shows it.
        assert_eq!(hold_target(1, Some(0), Some(2)), Some(2));
        assert_eq!(hold_target(1, Some(0), None), None);
        // Setup has no Current box.
        assert_eq!(hold_target(0, Some(0), Some(1)), Some(0));
        assert_eq!(hold_target(0, None, None), None);
    }

    #[test]
    fn show_what_i_type_ends_after_a_minute_without_a_key() {
        let now = Instant::now();
        assert!(!typed_view_expired(None, now));
        assert!(!typed_view_expired(Some(now), now));
        let just_under = TYPED_IDLE_LIMIT - Duration::from_millis(1);
        assert!(!typed_view_expired(Some(now), now + just_under));
        assert!(typed_view_expired(Some(now), now + TYPED_IDLE_LIMIT));
    }

    #[test]
    fn a_refusal_never_clears_the_current_box() {
        assert_eq!(refusal_targets(3, RefusalFocus::Keep), None);
        assert_eq!(
            refusal_targets(3, RefusalFocus::ClearAndFocusNew),
            Some((1..3, 1))
        );
        assert_eq!(
            refusal_targets(2, RefusalFocus::ClearAndFocusNew),
            Some((0..2, 0))
        );
        assert_eq!(
            refusal_targets(3, RefusalFocus::FocusConfirm),
            Some((2..2, 2))
        );
        assert_eq!(
            refusal_targets(1, RefusalFocus::ClearAndFocusRetype),
            Some((0..1, 0))
        );
        assert_eq!(refusal_targets(0, RefusalFocus::ClearAndFocusRetype), None);
        assert_eq!(refusal_targets(1, RefusalFocus::ClearAndFocusNew), None);
    }

    #[test]
    fn caps_lock_comes_first_and_only_where_capitals_matter() {
        assert_eq!(time_row_line(false, ""), "");
        assert_eq!(time_row_line(false, "Fake time."), "Fake time.");
        assert_eq!(time_row_line(true, ""), "Caps Lock is on.");
        assert_eq!(
            time_row_line(true, "Fake time."),
            "Caps Lock is on. Fake time."
        );
        assert!(caps_lock_matters(&credential(false)));
        assert!(caps_lock_matters(&new_password(false)));
        assert!(!caps_lock_matters(&credential(true)));
    }

    /// A debug build panics here if an override's types don't match AppKit's.
    #[test]
    fn the_native_control_classes_register() {
        KeyButton::class();
        HoldButton::class();
        RetypeEditor::class();
        RetypeCell::class();
        RetypeField::class();
    }

    /// Whether Look Up really stays shut takes a real gesture: this only shows the doors are ours.
    #[test]
    fn the_retype_editor_answers_look_up_itself() {
        let implementation = |class: &AnyClass, selector: Sel| {
            class
                .instance_method(selector)
                .map(|method| method.implementation() as *const ())
        };
        for selector in [
            sel!(quickLookWithEvent:),
            sel!(quickLookPreviewItems:),
            sel!(_immediateActionAnimationControllerForCharacterAtIndex:withRecognizer:),
            sel!(_revealAnimationControllerForCharacterAtIndex:),
        ] {
            let ours = implementation(RetypeEditor::class(), selector);
            assert!(ours.is_some());
            assert_ne!(ours, implementation(NSTextView::class(), selector));
        }
    }
}
