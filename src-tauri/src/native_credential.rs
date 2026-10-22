//! Native credential controls. Credentials and Recovery Kits never leave Rust/native UI.
//!
//! The coordinator owns authorization, expiry and cancellation. This adapter collects input
//! inside the existing protected main window. A reply is never an authentication token.
//! Entry points run on the UI thread. OS string copies are cleared where APIs permit;
//! their physical erasure cannot be promised.
use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

pub type Completion = Box<dyn FnOnce(Result<Reply, PromptError>) + Send + 'static>;
/// Validate the pending operation; true marks genuine input, false only revalidates.
pub type Activity = Box<dyn Fn(bool) -> Authorization + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authorization {
    Allowed,
    /// Authorization is temporarily busy. Hide plaintext and reject actions until revalidated.
    Retry,
    Denied,
}

#[derive(Default)]
struct PresentationState {
    suspended: bool,
    closed: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum PresentationUpdate {
    Keep,
    Clear,
    Restore,
    Close,
}

impl PresentationState {
    fn authorize(&mut self, authorization: Authorization) -> PresentationUpdate {
        if self.closed {
            return PresentationUpdate::Close;
        }
        match authorization {
            Authorization::Allowed if std::mem::take(&mut self.suspended) => {
                PresentationUpdate::Restore
            }
            Authorization::Allowed => PresentationUpdate::Keep,
            Authorization::Retry => {
                self.suspended = true;
                PresentationUpdate::Clear
            }
            Authorization::Denied => {
                self.closed = true;
                self.suspended = false;
                PresentationUpdate::Close
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RevealMode {
    Held,
    Timed,
}

/// A Wallet readout stays in native controls as well. It is never an IPC value.
pub struct Reveal {
    pub title: String,
    pub detail: String,
    pub value: Zeroizing<String>,
    pub mode: RevealMode,
}

enum Request {
    Entry(Prompt),
    Reveal {
        reveal: Reveal,
        extend: Option<Box<dyn Fn() -> Authorization + Send>>,
    },
}

enum Callback {
    Entry(Completion),
    Reveal(Box<dyn FnOnce() + Send>),
}

impl Callback {
    fn finish(self, result: Result<Reply, PromptError>) {
        match self {
            Self::Entry(completion) => completion(result),
            Self::Reveal(completion) => completion(),
        }
    }
}

impl Request {
    fn text(&self) -> (&str, &str) {
        match self {
            Self::Entry(prompt) => prompt.text(),
            Self::Reveal { reveal, .. } => (&reveal.title, &reveal.detail),
        }
    }
    fn labels(&self) -> Vec<&str> {
        match self {
            Self::Entry(prompt) => prompt.labels(),
            Self::Reveal { .. } => Vec::new(),
        }
    }
    fn reply(&self, fields: Vec<Zeroizing<String>>) -> Result<Reply, &'static str> {
        match self {
            Self::Entry(prompt) => prompt.reply(fields),
            Self::Reveal { .. } => Err("Close this native readout to finish."),
        }
    }
}

/// Display-only wrapping. Core clipboard writes keep the original value. Explicit line breaks
/// keep long unbroken keys inside a native nonselectable readout; control bytes are visible
/// escapes, so a Win32 NUL cannot silently truncate the readout. Rust copies wipe on drop.
fn readout_text(value: &str) -> (Zeroizing<String>, usize) {
    let mut text = Zeroizing::new(String::with_capacity(value.len()));
    let (mut columns, mut lines) = (0, 1usize);
    let mut append = |c: char| {
        if c == '\n' {
            text.push(c);
            columns = 0;
            lines = lines.saturating_add(1);
            return;
        }
        let width = if c.is_ascii() { 1 } else { 2 };
        if columns + width > 48 {
            text.push('\n');
            columns = 0;
            lines = lines.saturating_add(1);
        }
        text.push(c);
        columns += width;
    };
    for c in value.chars() {
        if c.is_control() && c != '\n' {
            for escaped in c.escape_debug() {
                append(escaped);
            }
        } else {
            append(c);
        }
    }
    (text, lines)
}

/// Labels and action text come from validated Rust state, never credential IPC.
pub enum Prompt {
    Credential {
        title: String,
        detail: String,
        label: String,
    },
    NewPassword {
        title: String,
        detail: String,
        current_password: bool,
    },
    RecoveryKit {
        title: String,
        detail: String,
        recovery: Zeroizing<String>,
    },
}

/// Deliberately not Debug, Clone, Serialize or an IPC type.
pub enum Reply {
    Credential(Zeroizing<String>),
    NewPassword {
        current: Option<Zeroizing<String>>,
        new: Zeroizing<String>,
    },
    RecoveryKitConfirmed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptError {
    Unavailable,
    Busy,
    Cancelled,
}

/// Present controls inside this exact main window. Never opens a separate window.
pub fn present(
    window: &tauri::WebviewWindow,
    id: u64,
    prompt: Prompt,
    activity: Activity,
    completion: Completion,
) -> Result<(), PromptError> {
    #[cfg(target_os = "macos")]
    return mac::present(
        window,
        id,
        Request::Entry(prompt),
        activity,
        Callback::Entry(completion),
    );
    #[cfg(windows)]
    return windows::present(
        window,
        id,
        Request::Entry(prompt),
        activity,
        Callback::Entry(completion),
    );
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (window, id, prompt, activity, completion);
        Err(PromptError::Unavailable)
    }
}

/// Display one already-authorized Wallet field natively. The coordinator must recheck the
/// operation immediately before calling this and cancel when its core reveal disappears.
pub fn present_reveal(
    window: &tauri::WebviewWindow,
    id: u64,
    reveal: Reveal,
    on_extend: Box<dyn Fn() -> Authorization + Send>,
    activity: Activity,
    completion: Box<dyn FnOnce() + Send>,
) -> Result<(), PromptError> {
    #[cfg(target_os = "macos")]
    return mac::present(
        window,
        id,
        Request::Reveal {
            reveal,
            extend: Some(on_extend),
        },
        activity,
        Callback::Reveal(completion),
    );
    #[cfg(windows)]
    return windows::present(
        window,
        id,
        Request::Reveal {
            reveal,
            extend: Some(on_extend),
        },
        activity,
        Callback::Reveal(completion),
    );
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (window, id, reveal, on_extend, activity, completion);
        Err(PromptError::Unavailable)
    }
}

/// Cancel only this request; cancellation never submits a wrong credential.
pub fn cancel(id: u64) -> bool {
    #[cfg(target_os = "macos")]
    return mac::cancel(id);
    #[cfg(windows)]
    return windows::cancel(id);
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = id;
        false
    }
}

impl Prompt {
    fn text(&self) -> (&str, &str) {
        match self {
            Self::Credential { title, detail, .. }
            | Self::NewPassword { title, detail, .. }
            | Self::RecoveryKit { title, detail, .. } => (title, detail),
        }
    }

    fn labels(&self) -> Vec<&str> {
        match self {
            Self::Credential { label, .. } => vec![label],
            Self::NewPassword {
                current_password, ..
            } => {
                let mut labels = Vec::new();
                if *current_password {
                    labels.push("Current Master Password");
                }
                labels.extend(["New Master Password", "Confirm new Master Password"]);
                labels
            }
            Self::RecoveryKit { .. } => vec!["Retype the last two groups of your Recovery Key"],
        }
    }

    fn reply(&self, mut fields: Vec<Zeroizing<String>>) -> Result<Reply, &'static str> {
        if fields.len() != self.labels().len() || fields.iter().any(|value| value.len() > 4096) {
            return Err("Use at most 4,096 bytes in each field.");
        }
        match self {
            Self::Credential { .. } => Ok(Reply::Credential(fields.remove(0))),
            Self::NewPassword {
                current_password, ..
            } => {
                let current = current_password.then(|| fields.remove(0));
                let new = fields.remove(0);
                let confirmation = fields.remove(0);
                if new.as_str() != confirmation.as_str() {
                    return Err("The new Master Passwords do not match.");
                }
                if !policy::assess(&new).acceptable {
                    return Err(
                        "Choose a stronger Master Password, or use a suggested passphrase.",
                    );
                }
                Ok(Reply::NewPassword { current, new })
            }
            Self::RecoveryKit { recovery, .. } => {
                let symbols = |text: &str| -> Zeroizing<String> {
                    Zeroizing::new(
                        text.chars()
                            .filter(|c| *c != '-' && !c.is_whitespace())
                            .map(|c| match c.to_ascii_uppercase() {
                                'O' => '0',
                                'I' | 'L' => '1',
                                other => other,
                            })
                            .collect(),
                    )
                };
                let expected = symbols(recovery);
                let typed = symbols(&fields.remove(0));
                if typed.len() != 8 || !expected.ends_with(typed.as_str()) {
                    return Err(
                        "Check the last two groups against the Recovery Kit you wrote down.",
                    );
                }
                Ok(Reply::RecoveryKitConfirmed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Authorization, PresentationState, PresentationUpdate, Prompt, Reply, readout_text,
    };
    use zeroize::Zeroizing;
    fn fields(values: &[&str]) -> Vec<Zeroizing<String>> {
        values
            .iter()
            .map(|value| Zeroizing::new((*value).to_owned()))
            .collect()
    }

    #[test]
    fn retry_clears_plaintext_until_one_successful_revalidation_restores_it() {
        let mut state = PresentationState::default();
        assert_eq!(
            state.authorize(Authorization::Allowed),
            PresentationUpdate::Keep
        );
        assert_eq!(
            state.authorize(Authorization::Retry),
            PresentationUpdate::Clear
        );
        assert_eq!(
            state.authorize(Authorization::Retry),
            PresentationUpdate::Clear
        );
        assert_eq!(
            state.authorize(Authorization::Allowed),
            PresentationUpdate::Restore
        );
        assert_eq!(
            state.authorize(Authorization::Allowed),
            PresentationUpdate::Keep
        );
    }

    #[test]
    fn denied_presentation_never_restores_plaintext() {
        let mut state = PresentationState::default();
        assert_eq!(
            state.authorize(Authorization::Retry),
            PresentationUpdate::Clear
        );
        assert_eq!(
            state.authorize(Authorization::Denied),
            PresentationUpdate::Close
        );
        assert_eq!(
            state.authorize(Authorization::Allowed),
            PresentationUpdate::Close
        );
        assert_eq!(
            state.authorize(Authorization::Retry),
            PresentationUpdate::Close
        );
    }

    #[test]
    fn recovery_confirmation_never_returns_recovery_text() {
        let prompt = Prompt::RecoveryKit {
            title: "Fake kit".into(),
            detail: String::new(),
            recovery: Zeroizing::new("FAKE-FAKE-FAKE-FAKE-FAKE-ABCD-EF01".into()),
        };
        assert!(prompt.reply(fields(&["WRNG-GRPS"])).is_err());
        assert!(matches!(
            prompt.reply(fields(&["abcd efOl"])),
            Ok(Reply::RecoveryKitConfirmed)
        ));
        assert!(prompt.reply(fields(&["FAKE-ABCD-EF01"])).is_err());
    }

    #[test]
    fn new_password_requires_strength_and_exact_confirmation() {
        let prompt = Prompt::NewPassword {
            title: "Fake setup".into(),
            detail: String::new(),
            current_password: true,
        };
        assert!(prompt.reply(fields(&["fake-old", "weak", "weak"])).is_err());
        assert!(
            prompt
                .reply(fields(&["fake-old", "fake-one", "fake-two"]))
                .is_err()
        );
        let suggestion = policy::suggest_passphrase();
        assert!(matches!(
            prompt.reply(fields(&["fake-old", &suggestion, &suggestion])),
            Ok(Reply::NewPassword {
                current: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn input_shape_and_size_are_bounded() {
        let prompt = Prompt::Credential {
            title: "Fake action".into(),
            detail: String::new(),
            label: "Master Password".into(),
        };
        assert!(prompt.reply(Vec::new()).is_err());
        assert!(prompt.reply(fields(&[&"x".repeat(4097)])).is_err());
    }

    #[test]
    fn native_readout_wraps_long_unicode_without_losing_characters() {
        let fake = "FAKE🔑é漢字".repeat(200);
        let (display, lines) = readout_text(&fake);
        assert!(lines > 4);
        assert_eq!(display.replace('\n', ""), fake);
        assert!(display.lines().all(|line| {
            line.chars()
                .map(|c| if c.is_ascii() { 1 } else { 2 })
                .sum::<usize>()
                <= 48
        }));
        let (controls, _) = readout_text("FAKE\0KEY\twith\rcontrol\nbytes");
        assert!(!controls.contains('\0'));
        assert!(controls.contains("\\0"));
        assert!(controls.contains("\\t"));
        assert!(controls.contains("\\r"));
    }
}
