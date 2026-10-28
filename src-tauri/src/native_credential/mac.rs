//! AppKit controls replace the content view of the already protected NSWindow. The original
//! WebView stays retained, but cannot receive input or draw over the native credential panel.
use std::cell::RefCell;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadOnly, class, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBezelStyle, NSButton, NSFont, NSResponder, NSScrollView,
    NSSecureTextField, NSTextField, NSView, NSWindow,
};
use objc2_foundation::{
    MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRunLoopCommonModes, NSSize,
    NSString, ns_string,
};
use zeroize::Zeroizing;

use super::{
    Activity, Authorization, Callback, PresentationState, PresentationUpdate, Prompt, PromptError,
    Reply, Request, RevealMode, readout_text,
};

thread_local! { static ACTIVE: RefCell<Option<Panel>> = const { RefCell::new(None) }; }

struct Panel {
    id: u64,
    window: Retained<NSWindow>,
    original: Retained<NSView>,
    original_focus: Option<Retained<NSResponder>>,
    _view: Retained<NSView>,
    _target: Retained<Target>,
    _authentication: Option<Retained<NSView>>,
    fields: Vec<Retained<NSSecureTextField>>,
    sensitive_labels: Vec<Retained<NSTextField>>,
    feedback: Retained<NSTextField>,
    prompt: Request,
    completion: Callback,
    activity: Option<Activity>,
    presentation: PresentationState,
    suggestion: Option<Zeroizing<String>>,
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
        fn submit(&self, _sender: Option<&AnyObject>) { if activity(true) { submit(); } }
        #[unsafe(method(cancel:))]
        fn cancel(&self, _sender: Option<&AnyObject>) { activity(true); finish(Err(PromptError::Cancelled)); }
        #[unsafe(method(suggest:))]
        fn suggest(&self, _sender: Option<&AnyObject>) { if activity(true) { suggest(); } }
        #[unsafe(method(extend:))]
        fn extend(&self, _sender: Option<&AnyObject>) { if activity(true) { extend(); } }
        #[unsafe(method(tick:))]
        fn tick(&self, _sender: Option<&AnyObject>) { tick(); }
        #[unsafe(method(focusLost:))]
        fn focus_lost(&self, _notification: &AnyObject) { finish(Err(PromptError::Cancelled)); }
        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn command(&self, _control: &AnyObject, _editor: &AnyObject, command: Sel) -> bool {
            if command == sel!(cancelOperation:) {
                activity(true); finish(Err(PromptError::Cancelled)); true }
            else if command == sel!(insertNewline:) { activity(true); submit(); true }
            else { false }
        }
        #[unsafe(method(controlTextDidChange:))]
        fn changed(&self, _notification: &AnyObject) {
            forget_suggestion();
            activity(edit_activity_is_genuine());
        }
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn editing(&self, _notification: &AnyObject) {
            ACTIVE.with(|active| {
                if let Some(panel) = active.borrow().as_ref() {
                    for field in &panel.fields {
                        // SAFETY: currentEditor is an optional NSTextView belonging to this
                        // live field. Only documented NSTextView assistance switches are sent.
                        unsafe {
                            let editor: Option<Retained<AnyObject>> = msg_send![field, currentEditor];
                            if let Some(editor) = editor {
                                let _: () = msg_send![&editor, setAllowsUndo: false];
                                let _: () = msg_send![&editor, setAutomaticSpellingCorrectionEnabled: false];
                                let _: () = msg_send![&editor, setAutomaticTextReplacementEnabled: false];
                                let _: () = msg_send![&editor, setAutomaticQuoteSubstitutionEnabled: false];
                                let _: () = msg_send![&editor, setAutomaticDashSubstitutionEnabled: false];
                                let _: () = msg_send![&editor, setAutomaticTextCompletionEnabled: false];
                                let _: () = msg_send![&editor, setContinuousSpellCheckingEnabled: false];
                                let _: () = msg_send![&editor, setGrammarCheckingEnabled: false];
                            }
                        }
                    }
                }
            });
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
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _event: &AnyObject) {
            if !activity(true) { return; }
            self.highlight(true);
            held(true);
        }
        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &AnyObject) {
            self.highlight(false);
            held(false);
        }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &AnyObject) {
            if !activity(!unsafe { msg_send![event, isARepeat] }) { return; }
            if reveal_navigation(event) { return; }
            // SAFETY: keyDown: receives an NSEvent and this is its documented string property.
            let text: Option<Retained<NSString>> = unsafe { msg_send![event, charactersIgnoringModifiers] };
            if text.is_some_and(|text| text.to_string() == " ") { held(true); }
            // SAFETY: Forward other navigation keys to NSButton's own implementation.
            else { unsafe { let _: () = msg_send![super(self), keyDown: event]; } }
        }
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &AnyObject) {
            if !activity(true) { return; }
            if is_space_event(event) { held(false); }
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
    let view = NSView::initWithFrame(NSView::alloc(mtm), original.bounds());
    view.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    let target = Target::new(mtm);
    let bounds = view.bounds();
    let width = (bounds.size.width - 64.0).clamp(240.0, 560.0);
    let x = (bounds.size.width - width) / 2.0;
    let mut y = bounds.size.height - 28.0;
    let (title, detail) = prompt.text();
    let heading = label(&view, title, x, place(&mut y, 30.0, 8.0), width, 30.0, mtm);
    heading.setFont(Some(&NSFont::boldSystemFontOfSize(21.0)));
    let detail_height = match &prompt {
        Request::Entry(Prompt::NewPassword { .. }) => 48.0,
        Request::Entry(Prompt::RecoveryKit { .. }) | Request::Biometric(_) => 64.0,
        _ => 160.0,
    };
    label(
        &view,
        detail,
        x,
        place(&mut y, detail_height, 8.0),
        width,
        detail_height,
        mtm,
    );
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
            let button = unsafe {
                msg_send![HoldButton::alloc(mtm), initWithFrame: rect(x, button_y, 230.0, 32.0)]
            };
            let button: Retained<HoldButton> = button;
            anchor(&button);
            button.setTitle(ns_string!("Hold to show (or hold Space)"));
            button.setBezelStyle(NSBezelStyle::Push);
            view.addSubview(&button);
            hold_button = Some(button);
        } else {
            extend_button = Some(button(
                &view,
                "Extend by 20 seconds",
                rect(x, button_y, 200.0, 30.0),
                &target,
                sel!(extend:),
                mtm,
            ));
        }
    }
    if let Request::Entry(Prompt::RecoveryKit { recovery, .. }) = &prompt {
        label(
            &view,
            "Write this Recovery Key on paper and store it somewhere private.",
            x,
            place(&mut y, 32.0, 6.0),
            width,
            32.0,
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
        key.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(16.0, 0.0)));
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
        let field = NSSecureTextField::initWithFrame(
            NSSecureTextField::alloc(mtm),
            rect(x, place(&mut y, 28.0, 8.0), width, 28.0),
        );
        anchor(&field);
        field.setFont(Some(&NSFont::systemFontOfSize(15.0)));
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
    let mut suggestion_button = None;
    if matches!(&prompt, Request::Entry(Prompt::NewPassword { .. })) {
        suggestion_button = Some(button(
            &view,
            "Suggest passphrase",
            rect(x, place(&mut y, 30.0, 4.0), 170.0, 30.0),
            &target,
            sel!(suggest:),
            mtm,
        ));
        let suggestion = label(&view, "", x, place(&mut y, 36.0, 4.0), width, 36.0, mtm);
        suggestion.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0)));
        sensitive_labels.push(suggestion);
    }
    let feedback = label(
        &view,
        feedback_text(&prompt),
        x,
        place(&mut y, 36.0, 6.0),
        width,
        36.0,
        mtm,
    );
    let button_y = place(&mut y, 30.0, 0.0);
    let cancel = button(
        &view,
        "Cancel",
        rect(x, button_y, 100.0, 30.0),
        &target,
        sel!(cancel:),
        mtm,
    );
    cancel.setKeyEquivalent(ns_string!("\u{1b}"));
    let is_reveal = matches!(&prompt, Request::Reveal { .. });
    // Touch ID has nothing to submit: the sensor is the only way forward.
    let accept = (!matches!(&prompt, Request::Biometric(_))).then(|| {
        let accept = button(
            &view,
            if is_reveal { "Close" } else { "Continue" },
            rect(x + width - 112.0, button_y, 112.0, 30.0),
            &target,
            if is_reveal {
                sel!(cancel:)
            } else {
                sel!(submit:)
            },
            mtm,
        );
        accept.setKeyEquivalent(ns_string!("\r"));
        accept
    });
    let mut key_views: Vec<&NSView> = Vec::new();
    for field in &fields {
        key_views.push(&**field);
    }
    if let Some(scroll) = &readout_scroll {
        key_views.push(&**scroll);
    }
    if let Some(button) = &suggestion_button {
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
    let expires =
        matches!(&prompt, Request::Reveal { reveal, .. } if reveal.mode == RevealMode::Timed)
            .then(|| Instant::now() + Duration::from_secs(20));
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
            _view: view.clone(),
            _target: target,
            _authentication: authentication,
            fields,
            sensitive_labels,
            feedback,
            prompt,
            completion,
            activity: Some(on_activity),
            presentation: PresentationState::default(),
            suggestion: None,
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
    window.setContentView(Some(&view));
    ACTIVE.with(|active| {
        if let Some(panel) = active.borrow().as_ref()
            && let Some(field) = panel.fields.first()
        {
            window.makeFirstResponder(Some(field));
        }
    });
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

fn feedback_text(prompt: &Request) -> &'static str {
    match prompt {
        Request::Biometric(_) => "Touch ID stays in this window. Switching apps or Escape cancels.",
        _ => "Credentials stay in native controls. Escape cancels.",
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

fn activity(genuine: bool) -> bool {
    // A repeated key revalidates without keeping Auto-lock alive.
    let genuine = genuine
        && !unsafe {
            let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
            let event: Option<Retained<AnyObject>> = msg_send![&app, currentEvent];
            event.is_some_and(|event| {
                let kind: usize = msg_send![&event, type];
                kind == 10 && msg_send![&event, isARepeat]
            })
        };
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
    let update = ACTIVE.with(|active| {
        let mut active = active.try_borrow_mut().ok()?;
        let panel = active.as_mut().filter(|panel| panel.id == id)?;
        let update = panel.presentation.authorize(authorization);
        match update {
            PresentationUpdate::Clear => {
                for label in &panel.sensitive_labels {
                    label.setStringValue(ns_string!(""));
                }
                if let (Some(readout), Some(scroll)) = (&panel.readout, &panel.readout_scroll) {
                    set_readout(readout, scroll, "");
                }
                panel.readout_shown = false;
                panel.feedback.setStringValue(ns_string!("Please wait…"));
            }
            PresentationUpdate::Restore => {
                if let Request::Entry(Prompt::RecoveryKit { recovery, .. }) = &panel.prompt
                    && let Some(label) = panel.sensitive_labels.first()
                {
                    label.setStringValue(&NSString::from_str(recovery));
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
                    label.setStringValue(&NSString::from_str(suggestion));
                }
                // Restore only a generated candidate, never a manually typed password.
                panel
                    .feedback
                    .setStringValue(&NSString::from_str(feedback_text(&panel.prompt)));
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

fn edit_activity_is_genuine() -> bool {
    // Only input events can renew an edit lease; programmatic notifications revalidate.
    // SAFETY: AppKit invokes the text-field delegate on the main thread; currentEvent
    // is an optional live NSEvent, and isARepeat is queried only for a key-down event.
    unsafe {
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        let event: Option<Retained<AnyObject>> = msg_send![&app, currentEvent];
        let Some(event) = event else {
            return false;
        };
        let kind: usize = msg_send![&event, type];
        let repeated = kind == 10 && msg_send![&event, isARepeat];
        edit_event_is_genuine(Some(kind), repeated)
    }
}

fn edit_event_is_genuine(kind: Option<usize>, repeated: bool) -> bool {
    // NSEventType: keyDown, or mouse buttons used by native context-menu Paste.
    matches!(kind, Some(1 | 2 | 3 | 4 | 25 | 26)) || kind == Some(10) && !repeated
}

fn anchor(view: &NSView) {
    // Keep controls centered and at the same distance from the top when the main
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
            .map(|field| Zeroizing::new(field.stringValue().to_string()))
            .collect();
        match panel.prompt.reply(values) {
            Ok(reply) => Some(reply),
            Err(message) => {
                panel.feedback.setStringValue(&NSString::from_str(message));
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
            let is_held = ACTIVE.with(|active| active.borrow().as_ref().is_some_and(|panel| matches!(&panel.prompt, Request::Reveal { reveal, .. } if reveal.mode == RevealMode::Held)));
            if is_held {
                held(true);
            }
            is_held
        }
        "\t" | "\u{19}" => {
            let window =
                ACTIVE.with(|active| active.borrow().as_ref().map(|panel| panel.window.clone()));
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
                let active = active.borrow();
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
        let authorization = hook();
        let present = ACTIVE.with(|active| {
            let mut active = active.borrow_mut();
            let Some(panel) = active.as_mut().filter(|panel| panel.id == id) else {
                return false;
            };
            if let Request::Reveal { extend, .. } = &mut panel.prompt {
                *extend = Some(hook);
            }
            if authorization == Authorization::Allowed {
                panel.extensions += 1;
                panel.expires = Some(Instant::now() + Duration::from_secs(20));
            }
            true
        });
        if present && !apply_authorization(id, authorization) {
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
        let text = format!(
            "Hides in {} seconds. {} extensions remaining.",
            remaining.as_secs() + 1,
            3 - panel.extensions
        );
        panel.feedback.setStringValue(&NSString::from_str(&text));
        remaining.is_zero()
    });
    if expired {
        finish(Err(PromptError::Cancelled));
    }
}

fn forget_suggestion() {
    ACTIVE.with(|active| {
        // Programmatic suggestion installation can trigger native editor notifications.
        // Its builder already holds this borrow and installs the new candidate afterward.
        if let Ok(mut active) = active.try_borrow_mut()
            && let Some(panel) = active.as_mut()
            && panel.suggestion.take().is_some()
            && let Some(label) = panel.sensitive_labels.last()
        {
            label.setStringValue(ns_string!(""));
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
        let string = NSString::from_str(&suggestion);
        for field in panel.fields.iter().rev().take(2) {
            field.setStringValue(&string);
        }
        if let Some(label) = panel.sensitive_labels.last() {
            label.setStringValue(&string);
        }
        panel.suggestion = Some(suggestion);
        panel.feedback.setStringValue(ns_string!(
            "Suggested passphrase meets the strength rules. Write it somewhere private."
        ));
    });
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
                let _: () = msg_send![&editor, setString: ns_string!("")];
            }
            let _: () = msg_send![field, setDelegate: Option::<&AnyObject>::None];
        }
        field.setStringValue(ns_string!(""));
    }
    for label in &panel.sensitive_labels {
        label.setStringValue(ns_string!(""));
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
    use super::edit_event_is_genuine;

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
}
