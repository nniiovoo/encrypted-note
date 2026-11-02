//! A line that holds typed text must not be readable through accessibility, where a secure box
//! gives nothing. AppKit views need the main thread and libtest never runs a test on it, so this
//! target has its own `main` (`harness = false` in Cargo.toml). Fake text only; no window opens.
//! An app reading another window on screen is the physical check in native-credential-design.md.
#![allow(unsafe_code)]

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    use encrypted_note_app::native_credential::typed_line;
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadOnly, msg_send};
    use objc2_app_kit::{NSTextField, NSView};
    use objc2_foundation::{MainThreadMarker, NSArray, NSPoint, NSRect, NSSize, NSString};

    const FAKE: &str = "FAKE-TYPED-TEXT";
    let mtm = MainThreadMarker::new().expect("main runs on the main thread");
    let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(400.0, 100.0));
    let fake = NSString::from_str(FAKE);
    // What accessibility finds under this view.
    let readable = |parent: &NSView| -> Vec<String> {
        // SAFETY: Documented NSAccessibility getters on live objects. accessibilityValue is an
        // NSString for a label.
        unsafe {
            let children: Option<Retained<NSArray<AnyObject>>> =
                msg_send![parent, accessibilityChildren];
            children.map_or_else(Vec::new, |children| {
                children
                    .iter()
                    .filter_map(|child| {
                        let value: Option<Retained<AnyObject>> =
                            msg_send![&*child, accessibilityValue];
                        Some(Retained::cast_unchecked::<NSString>(value?).to_string())
                    })
                    .collect()
            })
        }
    };

    // A plain label shows that the walk can find text, so the check below can fail.
    let plain_parent = NSView::initWithFrame(NSView::alloc(mtm), frame);
    let plain = NSTextField::labelWithString(&fake, mtm);
    plain_parent.addSubview(&plain);
    assert_eq!(readable(&plain_parent), [FAKE]);

    let parent = NSView::initWithFrame(NSView::alloc(mtm), frame);
    let line = typed_line(&parent, frame, mtm);
    assert!(readable(&parent).is_empty(), "nothing typed yet");
    line.setStringValue(&fake);
    assert_eq!(line.stringValue().to_string(), FAKE);
    assert!(
        readable(&parent).is_empty(),
        "the typed line is readable through accessibility"
    );
}
