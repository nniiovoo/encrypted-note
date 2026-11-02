//! Native credential controls. Credentials and Recovery Kits never leave Rust/native UI.
//!
//! The coordinator owns authorization, expiry and cancellation. This adapter collects input
//! inside the existing protected main window. A reply is never an authentication token.
//! Entry points run on the UI thread. OS string copies are cleared where APIs permit;
//! their physical erasure cannot be promised.
use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
#[doc(hidden)]
pub use mac::typed_line;

pub type Completion = Box<dyn FnOnce(Result<Reply, PromptError>) + Send + 'static>;
/// Validate the pending operation; true marks genuine input, false only revalidates.
pub type Activity = Box<dyn Fn(bool) -> Authorization + Send>;
/// Read-only, for the time row. Asking never counts as activity.
pub type TimeProbe = Box<dyn Fn() -> TimeLeft + Send>;
/// May typed text be drawn? Asking never counts as activity.
pub type MayShow = Box<dyn Fn() -> ShowCheck + Send>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowCheck {
    Allowed,
    /// A known sharing app runs, even after its Shield was dismissed, or the core is unusable.
    Refused,
    /// The core is locked for now, so nothing is known. Never a reason to start showing.
    Busy,
}

impl ShowCheck {
    /// (the controls stay usable, blank what they show) for a timer's answer, given whether they
    /// are usable now. A brief busy core changes nothing: a long one ends in `Clear`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn controls(self, usable: bool) -> (bool, bool) {
        match self {
            Self::Allowed => (true, false),
            Self::Refused => (false, true),
            Self::Busy => (usable, false),
        }
    }
}

const TOO_LONG: &str = "That is too long. Use fewer characters.";
const KIT_PAPER_NOTE: &str = "Write this key on paper. It can open your Vault, so keep the paper somewhere private. Don't save it as a photo, screenshot, file or note, or in a cloud folder like iCloud Drive, OneDrive or Dropbox.";
const SUGGEST_BUTTON: &str = "Suggest a password";
const RESET_BUTTON: &str = "Reset to 20 seconds";
const SUGGESTED_TEXT: &str = "Write this on paper now and keep the paper away from this computer. Switching apps closes this screen and this password will not be used.";
const SUGGESTION_REMOVED: &str = "You changed the suggested password, so it was removed. Start typing again, or press Suggest a password.";
const SUGGESTION_ASK: &str = "Did you write it down? Press Continue again to use it.";
const KIT_ARMED: &str = "If you cancel, the key shown here will never work. Press Cancel or Escape again to cancel, or keep typing.";
const CAPS_LOCK_TEXT: &str = "Caps Lock is on.";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SHOW_TYPING_LABEL: &str = "Show what I type";
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const HOLD_TO_SHOW_LABEL: &str = "Hold to show what I typed";
/// Only on a real refusal, never for a busy core. Names no app and nothing typed.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SHOW_REFUSED_TEXT: &str = "You can't see what you type while a screen sharing or recording app is running. Quit that app, then open this screen again.";

const MAX_FIELD_BYTES: usize = 4096;
const RECOVERY_SYMBOLS: usize = 28;
/// A core lock this brief blanks nothing where the owner is typing (see `rides_out_a_blip`).
const RETRY_GRACE: Duration = Duration::from_millis(350);
/// A second press this soon is the same press: a bounce, or a double-click on Cancel.
const ARM_KEY_GAP: Duration = Duration::from_millis(300);
const ARM_CLICK_GAP: Duration = Duration::from_millis(500);
const ARM_WINDOW: Duration = Duration::from_secs(10);
const LIVE_PAUSE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Authorization {
    Allowed,
    /// Authorization is temporarily busy. Reject actions until revalidated, and hide plaintext
    /// unless this is a blip the panel rides out (see `PresentationState::authorize`).
    Retry,
    Denied,
}

#[derive(Default)]
struct PresentationState {
    suspended: bool,
    closed: bool,
    last_allowed: Option<Instant>,
}

#[derive(Debug, PartialEq, Eq)]
enum PresentationUpdate {
    Keep,
    /// Paint nothing and reject actions.
    Hold,
    Clear,
    Restore,
    Close,
}

impl PresentationState {
    /// Only `grace` gets Hold, and only after a real Allowed: a first frame still blanks.
    fn authorize(
        &mut self,
        authorization: Authorization,
        now: Instant,
        grace: bool,
    ) -> PresentationUpdate {
        if self.closed {
            return PresentationUpdate::Close;
        }
        match authorization {
            Authorization::Allowed => {
                self.last_allowed = Some(now);
                if std::mem::take(&mut self.suspended) {
                    PresentationUpdate::Restore
                } else {
                    PresentationUpdate::Keep
                }
            }
            Authorization::Retry => {
                let blip = grace
                    && !self.suspended
                    && self.last_allowed.is_some_and(|allowed| {
                        now.saturating_duration_since(allowed) < RETRY_GRACE
                    });
                if blip {
                    return PresentationUpdate::Hold;
                }
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

/// What the first press of Cancel or Escape on the Recovery Kit does: it only asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArmDecision {
    Arm,
    Ignore,
    Cancel,
}

/// The adapter drops auto-repeat before calling `press`.
#[derive(Default)]
struct CancelArm {
    armed_at: Option<Instant>,
}

impl CancelArm {
    fn press(&mut self, now: Instant, from_key: bool) -> ArmDecision {
        let Some(armed_at) = self.armed_at else {
            self.armed_at = Some(now);
            return ArmDecision::Arm;
        };
        let since = now.saturating_duration_since(armed_at);
        if since < if from_key { ARM_KEY_GAP } else { ARM_CLICK_GAP } {
            ArmDecision::Ignore
        } else if since <= ARM_WINDOW {
            self.armed_at = None;
            ArmDecision::Cancel
        } else {
            self.armed_at = Some(now);
            ArmDecision::Arm
        }
    }

    fn disarm(&mut self) {
        self.armed_at = None;
    }

    fn expired(&self, now: Instant) -> bool {
        self.armed_at
            .is_some_and(|armed_at| now.saturating_duration_since(armed_at) > ARM_WINDOW)
    }
}

/// A generated password on screen. Kept together, so a new one or a forget resets the flag too.
struct Suggestion {
    text: Zeroizing<String>,
    acknowledged: bool,
}

/// The advice line under new-password boxes, shown at typing pauses. It never reads or scores
/// the Current box, and Continue always re-runs the real gate: nothing here approves anything.
#[derive(Default)]
struct LiveCheck {
    /// The last real edit that `line` hasn't seen yet.
    edited_at: Option<Instant>,
    generation: u64,
    /// The answer for the text of `generation`.
    strength: Option<bool>,
    running: bool,
    /// `line` wanted an answer while the one worker was busy.
    waiting: bool,
    slot: Arc<Mutex<Option<(u64, bool)>>>,
}

impl LiveCheck {
    /// Reads no text. Call for every edit except this code's own writes to the boxes. `index` is
    /// the edited box of `boxes`, when known. The strength answer is about New, so Confirm keeps
    /// it. Only a positive match counts: a box that can't be told apart forgets it.
    fn box_edited(&mut self, now: Instant, boxes: usize, index: Option<usize>) {
        if index.is_some() && index == boxes.checked_sub(1) {
            self.edited_at = Some(now);
        } else {
            self.edited(now);
        }
    }

    /// New's text changed, or may have.
    fn edited(&mut self, now: Instant) {
        self.edited_at = Some(now);
        self.generation = self.generation.wrapping_add(1);
        self.strength = None;
    }

    /// True once the typing has paused and `line` hasn't seen the latest edit.
    fn due(&self, now: Instant) -> bool {
        self.edited_at
            .is_some_and(|edited_at| now.saturating_duration_since(edited_at) >= LIVE_PAUSE)
    }

    fn line(&mut self, new: &str, confirm: &str) -> String {
        self.edited_at = None;
        if new.len() > MAX_FIELD_BYTES || confirm.len() > MAX_FIELD_BYTES {
            return TOO_LONG.to_owned();
        }
        let count = policy::char_count(new);
        let short = count < policy::MIN_CHARS;
        let mut line = if short {
            format!("{count} of {} characters", policy::MIN_CHARS)
        } else {
            match self.strength {
                Some(true) => "Strong enough.".to_owned(),
                Some(false) => "Not strong enough yet. Add another unusual word.".to_owned(),
                None => {
                    self.score(new);
                    "Checking…".to_owned()
                }
            }
        };
        if !confirm.is_empty() {
            line.push_str(if short { ". " } else { " " });
            line.push_str(if new == confirm {
                "The two passwords match."
            } else {
                "The two passwords don't match yet."
            });
        }
        line
    }

    /// True when `line` should run again: a fresh answer came, or the worker is free for text
    /// that had to wait. A stale answer is dropped here. Never waits and never counts as activity.
    fn poll(&mut self) -> bool {
        let Some((generation, acceptable)) =
            self.slot.try_lock().ok().and_then(|mut slot| slot.take())
        else {
            return false;
        };
        self.running = false;
        if generation == self.generation {
            self.strength = Some(acceptable);
            self.waiting = false;
            return true;
        }
        std::mem::take(&mut self.waiting) && self.edited_at.is_none()
    }

    /// At most one worker. It owns the only copy made here, and only a bool comes back.
    fn score(&mut self, new: &str) {
        if self.running {
            self.waiting = true;
            return;
        }
        let text = Zeroizing::new(new.to_owned());
        let (generation, slot) = (self.generation, Arc::clone(&self.slot));
        self.running = std::thread::Builder::new()
            .spawn(move || {
                let acceptable = policy::assess(&text).acceptable;
                drop(text);
                if let Ok(mut slot) = slot.lock() {
                    *slot = Some((generation, acceptable));
                }
            })
            .is_ok();
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
    /// The readout's own limit, set when the Master Password was accepted.
    pub closes_at: Instant,
}

/// Touch ID inside the protected window: an explanation, the system's embedded biometric view
/// for one biometry request, and Cancel. It collects nothing and returns no value.
pub struct Biometric {
    pub title: String,
    pub detail: String,
    /// The biometry request whose paired authentication context the embedded view presents.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub request: u64,
}

enum Request {
    Entry(Prompt),
    Reveal {
        reveal: Reveal,
        extend: Option<Box<dyn Fn() -> Authorization + Send>>,
    },
    Biometric(Biometric),
}

enum Callback {
    Entry(Completion),
    Reveal(Box<dyn FnOnce() + Send>),
    Biometric(Box<dyn FnOnce() + Send>),
}

impl Callback {
    fn finish(self, result: Result<Reply, PromptError>) {
        match self {
            Self::Entry(completion) => completion(result),
            Self::Reveal(completion) | Self::Biometric(completion) => completion(),
        }
    }
}

impl Request {
    /// The Recovery Key retype box shows what is typed: the key it repeats is on screen right
    /// above it, and typing blind from paper is needlessly hard. Every other input stays a
    /// secure field; `offers_show_typing` and `offers_hold_to_show` only add a separate mirror.
    fn shows_typing(&self) -> bool {
        matches!(self, Self::Entry(Prompt::RecoveryKit { .. }))
    }
    fn is_kit(&self) -> bool {
        matches!(self, Self::Entry(Prompt::RecoveryKit { .. }))
    }
    #[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
    fn offers_show_typing(&self) -> bool {
        matches!(self, Self::Entry(Prompt::Credential { recovery: true, .. }))
    }
    #[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
    fn offers_hold_to_show(&self) -> bool {
        matches!(self, Self::Entry(Prompt::NewPassword { .. }))
    }
    /// A brief core lock must not disturb these: the owner is typing, and on Mac an opt-in view
    /// would switch itself off.
    #[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
    fn rides_out_a_blip(&self) -> bool {
        self.is_kit() || self.offers_show_typing() || self.offers_hold_to_show()
    }
    /// Windows, where a rejected key never reaches the box. While the core is busy, actions wait
    /// but typing still lands: a missing key in a box of dots can't be seen, and it changes the
    /// password. Not in a blanked Kit box, because that step's key is hidden for now. The Mac's
    /// field editor takes the key before anything checks it.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn box_takes_typing_while_busy(&self, suspended: bool) -> bool {
        matches!(self, Self::Entry(_)) && !(self.is_kit() && suspended)
    }
    fn text(&self) -> (&str, &str) {
        match self {
            Self::Entry(prompt) => prompt.text(),
            Self::Reveal { reveal, .. } => (&reveal.title, &reveal.detail),
            Self::Biometric(biometric) => (&biometric.title, &biometric.detail),
        }
    }
    fn labels(&self) -> Vec<&str> {
        match self {
            Self::Entry(prompt) => prompt.labels(),
            Self::Reveal { .. } | Self::Biometric(_) => Vec::new(),
        }
    }
    fn reply(&self, fields: Vec<Zeroizing<String>>) -> Result<Reply, Refusal> {
        match self {
            Self::Entry(prompt) => prompt.reply(fields),
            Self::Reveal { .. } => Err(Refusal::keep("Press Close to finish.")),
            Self::Biometric(_) => Err(Refusal::keep("Use Touch ID, or choose Cancel.")),
        }
    }
    fn feedback_text(&self) -> String {
        let key = if cfg!(windows) { "Enter" } else { "Return" };
        match self {
            Self::Biometric(_) => {
                "Touch the sensor now. Switching to another app or pressing Escape cancels."
                    .to_owned()
            }
            Self::Entry(Prompt::RecoveryKit { .. }) => {
                format!(
                    "You can see what you type here. Press {key} to finish. Press Esc twice to cancel."
                )
            }
            Self::Entry(Prompt::NewPassword { .. }) => {
                format!("{key} moves to the next box, then finishes. Esc cancels.")
            }
            Self::Entry(Prompt::Credential { .. }) => {
                format!(
                    "Press {key} to continue, or Esc to cancel. Switching to another app clears this box."
                )
            }
            Self::Reveal { .. } => "Press Esc or Close when you finish.".to_owned(),
        }
    }
    /// None leaves the row as it is. An empty text clears it.
    fn time_row_text(&self, left: &TimeLeft) -> Option<String> {
        let (seconds, limit) = match *left {
            TimeLeft::Busy => return None,
            TimeLeft::Quiet => return Some(String::new()),
            TimeLeft::KitCalm { idle_minutes } => {
                return Some(if self.is_kit() {
                    kit_calm_text(idle_minutes)
                } else {
                    String::new()
                });
            }
            TimeLeft::Left { seconds, limit } => (seconds, limit),
        };
        let time = seconds_text(seconds);
        Some(match self {
            Self::Entry(Prompt::Credential { .. } | Prompt::NewPassword { .. }) => match limit {
                TimeLimit::Panel => {
                    format!("This box closes in {time}. You would need to type again.")
                }
                TimeLimit::AutoLock => format!("The app locks in {time} if you stop typing."),
            },
            Self::Entry(Prompt::RecoveryKit { rotation, .. }) => {
                let outcome = if *rotation {
                    "nothing changes: your old password and old Recovery Key still work, and this new key is never used."
                } else {
                    "no Vault is made and this key is never used."
                };
                match limit {
                    TimeLimit::Panel => format!("This step ends in {time}. Then {outcome}"),
                    // "If it locks": a "Then" here would read as the result of typing.
                    TimeLimit::AutoLock => format!(
                        "The app locks in {time}. Typing in the box above gives you more time. If it locks, {outcome}"
                    ),
                }
            }
            Self::Reveal { reveal, .. } if reveal.mode == RevealMode::Held => format!(
                "This screen closes in {time}. You will need your Master Password to show it again."
            ),
            Self::Reveal { .. } | Self::Biometric(_) => String::new(),
        })
    }
}

fn seconds_text(seconds: u64) -> String {
    if seconds == 1 {
        "1 second".to_owned()
    } else {
        format!("{seconds} seconds")
    }
}

/// States the rules, not the time left: this line stays up until the last minute.
fn kit_calm_text(idle_minutes: u32) -> String {
    match idle_minutes {
        1 => {
            "If you don't type for 1 minute, the app locks. This step ends after 10 minutes in all."
                .to_owned()
        }
        2..=9 => format!(
            "If you don't type for {idle_minutes} minutes, the app locks. This step ends after 10 minutes in all."
        ),
        _ => "This step ends after 10 minutes in all.".to_owned(),
    }
}

/// `left` is how many resets remain.
fn timed_readout_text(seconds: u64, left: u8) -> String {
    let time = seconds_text(seconds);
    match left {
        0 => format!(
            "Hides in {time}. You can't reset the timer again. Enter your Master Password to show it again."
        ),
        1 => format!("Hides in {time}. You can reset the timer 1 more time."),
        left => format!("Hides in {time}. You can reset the timer {left} more times."),
    }
}

/// Every refusal goes through this, so color is never the only sign of a problem.
fn problem_text(message: &str) -> String {
    format!("Problem: {message}")
}

/// The same filter as `vault_format::RecoveryKey::parse`.
fn recovery_symbols(value: &str) -> impl Iterator<Item = char> + '_ {
    value.chars().filter(|c| *c != '-' && !c.is_whitespace())
}

/// Holds a count only.
pub fn recovery_progress(value: &str) -> String {
    format!(
        "{} of {RECOVERY_SYMBOLS} characters",
        recovery_symbols(value).count()
    )
}

/// For the opt-in mirror: what the app reads as a Recovery Key, grouped like the paper. Anything
/// that can't be in one shows as `?`, so a password typed here by mistake doesn't show whole.
pub fn recovery_mirror(value: &str) -> Zeroizing<String> {
    // Reserved up front: growing would leave an unwiped copy behind.
    let mut mirror = Zeroizing::new(String::with_capacity(
        RECOVERY_SYMBOLS + RECOVERY_SYMBOLS / 4,
    ));
    for (index, symbol) in recovery_symbols(value).take(RECOVERY_SYMBOLS).enumerate() {
        if index > 0 && index % 4 == 0 {
            mirror.push('-');
        }
        mirror.push(match symbol.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            'U' => '?',
            upper @ ('0'..='9' | 'A'..='Z') => upper,
            _ => '?',
        });
    }
    mirror
}

/// Which box takes the focus after a refusal that keeps the boxes, when the focus isn't in one:
/// `empty` says which boxes are empty, in order. The first empty box is what the message asks
/// for. Otherwise New, where a weak password gets its next word, or the only box.
#[cfg_attr(not(any(target_os = "macos", windows)), allow(dead_code))]
fn kept_refusal_box(empty: &[bool]) -> Option<usize> {
    empty
        .iter()
        .position(|empty| *empty)
        .or_else(|| (!empty.is_empty()).then(|| empty.len().saturating_sub(2)))
}

/// Local checks only: no key derivation runs and no attempt is counted.
fn check_recovery_key(value: &str) -> Result<(), Refusal> {
    let wrong_length = || {
        Refusal::keep(format!(
            "A Recovery Key has {RECOVERY_SYMBOLS} characters. You typed {}.",
            recovery_symbols(value).count()
        ))
    };
    vault_format::RecoveryKey::parse(value)
        .map(drop)
        .map_err(|error| match error {
            vault_format::RecoveryKeyError::Length => wrong_length(),
            vault_format::RecoveryKeyError::InvalidSymbol(at) if at >= RECOVERY_SYMBOLS => {
                wrong_length()
            }
            vault_format::RecoveryKeyError::InvalidSymbol(at) => Refusal::keep(format!(
                "Group {} has a character a Recovery Key can't contain. Check it on your paper.",
                at / 4 + 1
            )),
            vault_format::RecoveryKeyError::Checksum => Refusal::keep(
                "Something in this key is wrong. Check each group against your paper.",
            ),
        })
}

/// The first hint is all the row holds. A hint with no next step gets one.
fn weak_password_text(assessment: &policy::Assessment) -> String {
    match assessment.feedback.first() {
        Some(hint) if hint.contains(SUGGEST_BUTTON) => hint.clone(),
        Some(hint) => format!("{hint} Add another unusual word, or press {SUGGEST_BUTTON}."),
        None => format!(
            "Choose a longer password. 5 or 6 random words work well, or press {SUGGEST_BUTTON}."
        ),
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
        recovery: bool,
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
        rotation: bool,
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

/// Why a panel stays open. Holds counts and group numbers only, never anything typed.
struct Refusal {
    message: Cow<'static, str>,
    focus: RefusalFocus,
}

impl Refusal {
    fn new(message: impl Into<Cow<'static, str>>, focus: RefusalFocus) -> Self {
        Self {
            message: message.into(),
            focus,
        }
    }
    fn keep(message: impl Into<Cow<'static, str>>) -> Self {
        Self::new(message, RefusalFocus::Keep)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefusalFocus {
    Keep,
    /// Clears New and Confirm, never Current.
    ClearAndFocusNew,
    FocusConfirm,
    /// Never select the text instead: a selection can reach the ordinary clipboard.
    ClearAndFocusRetype,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptError {
    Unavailable,
    Busy,
    /// The owner's Cancel button or Escape, or the coordinator closing this exact request.
    Cancelled,
    /// The panel closed by itself: another app took focus, or authorization was denied.
    Interrupted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeLimit {
    Panel,
    AutoLock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeLeft {
    /// Can't tell right now: leave the row as it is.
    Busy,
    Quiet,
    /// Recovery Kit with time to spare.
    KitCalm {
        idle_minutes: u32,
    },
    Left {
        seconds: u64,
        limit: TimeLimit,
    },
}

/// Present controls inside this exact main window. Never opens a separate window.
pub fn present(
    window: &tauri::WebviewWindow,
    id: u64,
    prompt: Prompt,
    activity: Activity,
    time_left: TimeProbe,
    may_show: MayShow,
    completion: Completion,
) -> Result<(), PromptError> {
    #[cfg(target_os = "macos")]
    return mac::present(
        window,
        id,
        Request::Entry(prompt),
        activity,
        time_left,
        may_show,
        Callback::Entry(completion),
    );
    #[cfg(windows)]
    return windows::present(
        window,
        id,
        Request::Entry(prompt),
        activity,
        time_left,
        may_show,
        Callback::Entry(completion),
    );
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (
            window, id, prompt, activity, time_left, may_show, completion,
        );
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
    time_left: TimeProbe,
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
        time_left,
        Box::new(|| ShowCheck::Refused),
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
        time_left,
        Box::new(|| ShowCheck::Refused),
        Callback::Reveal(completion),
    );
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = (
            window, id, reveal, on_extend, activity, time_left, completion,
        );
        Err(PromptError::Unavailable)
    }
}

/// Show Touch ID inside this exact main window (Mac only). The completion runs once when the
/// panel closes for any reason; the caller cancels its biometry request then, which also
/// invalidates the paired context, so authentication never continues without this panel.
pub fn present_biometric(
    window: &tauri::WebviewWindow,
    id: u64,
    biometric: Biometric,
    activity: Activity,
    completion: Box<dyn FnOnce() + Send>,
) -> Result<(), PromptError> {
    let request = Request::Biometric(biometric);
    let callback = Callback::Biometric(completion);
    #[cfg(target_os = "macos")]
    return mac::present(
        window,
        id,
        request,
        activity,
        Box::new(|| TimeLeft::Quiet),
        Box::new(|| ShowCheck::Refused),
        callback,
    );
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, id, request, activity, callback);
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
                labels.extend(["New Master Password", "Type the new Master Password again"]);
                labels
            }
            Self::RecoveryKit { .. } => vec!["Type the last two groups of 4 from your paper"],
        }
    }

    fn reply(&self, mut fields: Vec<Zeroizing<String>>) -> Result<Reply, Refusal> {
        if fields.len() != self.labels().len()
            || fields.iter().any(|value| value.len() > MAX_FIELD_BYTES)
        {
            return Err(Refusal::keep(TOO_LONG));
        }
        match self {
            Self::Credential { recovery, .. } => {
                let value = fields.remove(0);
                // Empty only, never trimmed: a space is a real attempt.
                if value.is_empty() {
                    return Err(Refusal::keep(if *recovery {
                        "Type your Recovery Key first."
                    } else {
                        "Type your Master Password first."
                    }));
                }
                if *recovery {
                    check_recovery_key(&value)?;
                }
                Ok(Reply::Credential(value))
            }
            Self::NewPassword {
                current_password, ..
            } => {
                let current = current_password.then(|| fields.remove(0));
                if current.as_ref().is_some_and(|current| current.is_empty()) {
                    return Err(Refusal::keep("Type your current Master Password first."));
                }
                let new = fields.remove(0);
                let confirmation = fields.remove(0);
                if confirmation.is_empty() && !new.is_empty() {
                    return Err(Refusal::new(
                        "Type the same password in the second box.",
                        RefusalFocus::FocusConfirm,
                    ));
                }
                if new.as_str() != confirmation.as_str() {
                    return Err(Refusal::new(
                        "The two passwords are different. Type both again.",
                        RefusalFocus::ClearAndFocusNew,
                    ));
                }
                let assessment = policy::assess(&new);
                if !assessment.acceptable {
                    return Err(Refusal::keep(weak_password_text(&assessment)));
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
                    return Err(Refusal::new(
                        "That doesn't match. Look at the last two groups on your paper and type them again.",
                        RefusalFocus::ClearAndFocusRetype,
                    ));
                }
                Ok(Reply::RecoveryKitConfirmed)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ArmDecision, Authorization, Biometric, CancelArm, LiveCheck, PresentationState,
        PresentationUpdate, Prompt, Refusal, RefusalFocus, Reply, Request, Reveal, RevealMode,
        ShowCheck, TimeLeft, TimeLimit, kept_refusal_box, problem_text, readout_text,
        recovery_mirror, recovery_progress, timed_readout_text, weak_password_text,
    };
    use std::time::{Duration, Instant};
    use zeroize::Zeroizing;
    fn fields(values: &[&str]) -> Vec<Zeroizing<String>> {
        values
            .iter()
            .map(|value| Zeroizing::new((*value).to_owned()))
            .collect()
    }
    fn refused(result: Result<Reply, Refusal>) -> Refusal {
        match result {
            Ok(_) => panic!("expected the panel to stay open"),
            Err(refusal) => refusal,
        }
    }
    fn credential(recovery: bool) -> Prompt {
        Prompt::Credential {
            title: "Fake action".into(),
            detail: String::new(),
            label: "Fake label".into(),
            recovery,
        }
    }
    fn new_password(current_password: bool) -> Prompt {
        Prompt::NewPassword {
            title: "Fake setup".into(),
            detail: String::new(),
            current_password,
        }
    }
    fn kit(rotation: bool) -> Prompt {
        Prompt::RecoveryKit {
            title: "Fake kit".into(),
            detail: String::new(),
            recovery: Zeroizing::new("FAKE-FAKE-FAKE-FAKE-FAKE-ABCD-EF01".into()),
            rotation,
        }
    }
    fn readout(mode: RevealMode) -> Request {
        Request::Reveal {
            reveal: Reveal {
                title: "Fake readout".into(),
                detail: String::new(),
                value: Zeroizing::new("fake-value".into()),
                mode,
                closes_at: Instant::now(),
            },
            extend: None,
        }
    }
    fn biometric() -> Request {
        Request::Biometric(Biometric {
            title: "Fake".into(),
            detail: String::new(),
            request: 1,
        })
    }
    fn left(seconds: u64, limit: TimeLimit) -> TimeLeft {
        TimeLeft::Left { seconds, limit }
    }

    #[test]
    fn retry_clears_plaintext_until_one_successful_revalidation_restores_it() {
        let mut state = PresentationState::default();
        let now = Instant::now();
        assert_eq!(
            state.authorize(Authorization::Allowed, now, false),
            PresentationUpdate::Keep
        );
        assert_eq!(
            state.authorize(Authorization::Retry, now, false),
            PresentationUpdate::Clear
        );
        assert_eq!(
            state.authorize(Authorization::Retry, now, false),
            PresentationUpdate::Clear
        );
        assert_eq!(
            state.authorize(Authorization::Allowed, now, false),
            PresentationUpdate::Restore
        );
        assert_eq!(
            state.authorize(Authorization::Allowed, now, false),
            PresentationUpdate::Keep
        );
    }

    #[test]
    fn denied_presentation_never_restores_plaintext() {
        let mut state = PresentationState::default();
        let now = Instant::now();
        assert_eq!(
            state.authorize(Authorization::Retry, now, false),
            PresentationUpdate::Clear
        );
        assert_eq!(
            state.authorize(Authorization::Denied, now, false),
            PresentationUpdate::Close
        );
        assert_eq!(
            state.authorize(Authorization::Allowed, now, false),
            PresentationUpdate::Close
        );
        assert_eq!(
            state.authorize(Authorization::Retry, now, false),
            PresentationUpdate::Close
        );
    }

    #[test]
    fn a_brief_retry_on_the_kit_paints_nothing_but_only_after_a_real_allowed() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);

        // No grace on a first frame.
        let mut state = PresentationState::default();
        assert_eq!(
            state.authorize(Authorization::Retry, at(0), true),
            PresentationUpdate::Clear
        );
        assert_eq!(
            state.authorize(Authorization::Allowed, at(10), true),
            PresentationUpdate::Restore
        );

        // Under the limit it holds, and holding doesn't move the limit.
        assert_eq!(
            state.authorize(Authorization::Retry, at(100), true),
            PresentationUpdate::Hold
        );
        assert_eq!(
            state.authorize(Authorization::Retry, at(359), true),
            PresentationUpdate::Hold
        );
        assert_eq!(
            state.authorize(Authorization::Retry, at(360), true),
            PresentationUpdate::Clear
        );
        // Blanked stays blanked until a real Allowed, however near the last one was.
        assert_eq!(
            state.authorize(Authorization::Retry, at(361), true),
            PresentationUpdate::Clear
        );
        assert_eq!(
            state.authorize(Authorization::Allowed, at(400), true),
            PresentationUpdate::Restore
        );

        // A hold had nothing to restore.
        assert_eq!(
            state.authorize(Authorization::Retry, at(500), true),
            PresentationUpdate::Hold
        );
        assert_eq!(
            state.authorize(Authorization::Allowed, at(520), true),
            PresentationUpdate::Keep
        );

        // Denied wins.
        assert_eq!(
            state.authorize(Authorization::Retry, at(530), true),
            PresentationUpdate::Hold
        );
        assert_eq!(
            state.authorize(Authorization::Denied, at(531), true),
            PresentationUpdate::Close
        );
        assert_eq!(
            state.authorize(Authorization::Retry, at(532), true),
            PresentationUpdate::Close
        );

        // Other prompts behave as before.
        let mut state = PresentationState::default();
        assert_eq!(
            state.authorize(Authorization::Allowed, at(0), false),
            PresentationUpdate::Keep
        );
        assert_eq!(
            state.authorize(Authorization::Retry, at(1), false),
            PresentationUpdate::Clear
        );
    }

    #[test]
    fn only_panels_where_the_owner_types_ride_out_a_blip() {
        for request in [
            Request::Entry(kit(false)),
            Request::Entry(credential(true)),
            Request::Entry(new_password(false)),
            Request::Entry(new_password(true)),
        ] {
            assert!(request.rides_out_a_blip());
        }
        for request in [
            Request::Entry(credential(false)),
            readout(RevealMode::Held),
            readout(RevealMode::Timed),
            biometric(),
        ] {
            assert!(!request.rides_out_a_blip());
        }
    }

    #[test]
    fn typing_lands_in_a_box_while_the_core_is_busy_except_a_blanked_kit() {
        for request in [
            Request::Entry(credential(false)),
            Request::Entry(credential(true)),
            Request::Entry(new_password(false)),
            Request::Entry(new_password(true)),
        ] {
            assert!(request.box_takes_typing_while_busy(false));
            assert!(request.box_takes_typing_while_busy(true));
        }
        assert!(Request::Entry(kit(false)).box_takes_typing_while_busy(false));
        assert!(!Request::Entry(kit(false)).box_takes_typing_while_busy(true));
        for request in [readout(RevealMode::Held), biometric()] {
            assert!(!request.box_takes_typing_while_busy(false));
        }
    }

    #[test]
    fn a_busy_answer_changes_nothing_and_only_a_refusal_blanks() {
        assert_eq!(ShowCheck::Allowed.controls(false), (true, false));
        assert_eq!(ShowCheck::Refused.controls(true), (false, true));
        // A first frame starts unusable, so a busy one stays a no.
        assert_eq!(ShowCheck::Busy.controls(false), (false, false));
        assert_eq!(ShowCheck::Busy.controls(true), (true, false));
    }

    #[test]
    fn a_kept_refusal_focuses_the_box_it_asks_for() {
        assert_eq!(kept_refusal_box(&[]), None);
        // The only box: Master Password or Recovery Key, empty or not.
        assert_eq!(kept_refusal_box(&[true]), Some(0));
        assert_eq!(kept_refusal_box(&[false]), Some(0));
        // Type your current Master Password first, with New and Confirm filled.
        assert_eq!(kept_refusal_box(&[true, false, false]), Some(0));
        // Nothing typed yet is a length problem, and New comes first.
        assert_eq!(kept_refusal_box(&[true, true]), Some(0));
        // A weak password: its next word goes in New.
        assert_eq!(kept_refusal_box(&[false, false, false]), Some(1));
        assert_eq!(kept_refusal_box(&[false, false]), Some(0));
    }

    #[test]
    fn only_the_recovery_key_retype_shows_what_is_typed() {
        assert!(Request::Entry(kit(false)).shows_typing());
        assert!(Request::Entry(kit(false)).is_kit());
        assert!(!Request::Entry(credential(false)).shows_typing());
        assert!(!Request::Entry(credential(true)).shows_typing());
        assert!(!Request::Entry(credential(true)).is_kit());
        assert!(!Request::Entry(new_password(true)).shows_typing());
        assert!(!biometric().shows_typing());
    }

    #[test]
    fn plaintext_mirrors_are_offered_only_where_the_owner_chose_them() {
        assert!(Request::Entry(credential(true)).offers_show_typing());
        for request in [
            Request::Entry(credential(false)),
            Request::Entry(new_password(true)),
            Request::Entry(kit(false)),
            readout(RevealMode::Held),
            biometric(),
        ] {
            assert!(!request.offers_show_typing());
        }
        assert!(Request::Entry(new_password(false)).offers_hold_to_show());
        assert!(Request::Entry(new_password(true)).offers_hold_to_show());
        for request in [
            Request::Entry(credential(false)),
            Request::Entry(credential(true)),
            Request::Entry(kit(false)),
            readout(RevealMode::Timed),
            biometric(),
        ] {
            assert!(!request.offers_hold_to_show());
        }
    }

    #[test]
    fn recovery_confirmation_never_returns_recovery_text() {
        let prompt = kit(false);
        assert!(prompt.reply(fields(&["WRNG-GRPS"])).is_err());
        assert!(matches!(
            prompt.reply(fields(&["abcd efOl"])),
            Ok(Reply::RecoveryKitConfirmed)
        ));
        let refusal = refused(prompt.reply(fields(&["FAKE-ABCD-EF01"])));
        assert_eq!(refusal.focus, RefusalFocus::ClearAndFocusRetype);
        assert_eq!(
            refusal.message,
            "That doesn't match. Look at the last two groups on your paper and type them again."
        );
    }

    #[test]
    fn new_password_requires_strength_and_exact_confirmation() {
        let prompt = new_password(true);
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
    fn new_password_refusals_say_what_to_do_and_where_to_go() {
        let prompt = new_password(true);
        let fake = policy::suggest_passphrase();

        let refusal = refused(prompt.reply(fields(&["", &fake, &fake])));
        assert_eq!(refusal.message, "Type your current Master Password first.");
        assert_eq!(refusal.focus, RefusalFocus::Keep);

        let refusal = refused(prompt.reply(fields(&["fake-old", &fake, ""])));
        assert_eq!(refusal.message, "Type the same password in the second box.");
        assert_eq!(refusal.focus, RefusalFocus::FocusConfirm);

        let refusal = refused(prompt.reply(fields(&["fake-old", &fake, "fake-other"])));
        assert_eq!(
            refusal.message,
            "The two passwords are different. Type both again."
        );
        assert_eq!(refusal.focus, RefusalFocus::ClearAndFocusNew);

        let refusal = refused(prompt.reply(fields(&["fake-old", "fake-short", "fake-short"])));
        assert_eq!(
            refusal.message,
            "Use at least 15 characters. 5 or 6 random words work well, or press Suggest a password."
        );
        assert_eq!(refusal.focus, RefusalFocus::Keep);
        assert!(!refusal.message.contains("fake-short"));

        // Nothing typed yet is a length problem, not a mismatch.
        let refusal = refused(new_password(false).reply(fields(&["", ""])));
        assert!(refusal.message.contains("15 characters"));
        assert_eq!(refusal.focus, RefusalFocus::Keep);
    }

    #[test]
    fn a_hint_without_a_next_step_gets_one() {
        let hints = |feedback: &[&str]| policy::Assessment {
            acceptable: false,
            score: 0,
            feedback: feedback.iter().map(|hint| (*hint).to_owned()).collect(),
        };
        assert_eq!(
            weak_password_text(&hints(&[
                "Fake hint, or press Suggest a password.",
                "Ignored"
            ])),
            "Fake hint, or press Suggest a password."
        );
        assert_eq!(
            weak_password_text(&hints(&["Fake warning.", "Ignored"])),
            "Fake warning. Add another unusual word, or press Suggest a password."
        );
        assert_eq!(
            weak_password_text(&hints(&[])),
            "Choose a longer password. 5 or 6 random words work well, or press Suggest a password."
        );
    }

    #[test]
    fn an_empty_box_is_refused_but_a_space_is_a_real_attempt() {
        assert_eq!(
            refused(credential(false).reply(fields(&[""]))).message,
            "Type your Master Password first."
        );
        assert_eq!(
            refused(credential(true).reply(fields(&[""]))).message,
            "Type your Recovery Key first."
        );
        assert!(matches!(
            credential(false).reply(fields(&[" "])),
            Ok(Reply::Credential(_))
        ));
    }

    #[test]
    fn recovery_key_typos_keep_the_panel_and_name_only_counts_and_groups() {
        let prompt = credential(true);
        let typed = "QQQ";
        let refusal = refused(prompt.reply(fields(&[typed])));
        assert_eq!(
            refusal.message,
            "A Recovery Key has 28 characters. You typed 3."
        );
        assert_eq!(refusal.focus, RefusalFocus::Keep);
        // Spaces and dashes aren't characters of the key.
        assert_eq!(
            refused(prompt.reply(fields(&[" - "]))).message,
            "A Recovery Key has 28 characters. You typed 0."
        );

        let bad_symbol = "QQQQ-QQQQ-QQUQ-QQQQ-QQQQ-QQQQ-QQQQ";
        assert_eq!(
            refused(prompt.reply(fields(&[bad_symbol]))).message,
            "Group 3 has a character a Recovery Key can't contain. Check it on your paper."
        );
        let too_long = "QQQQ-QQQQ-QQQQ-QQQQ-QQQQ-QQQQ-QQQQ-U";
        assert_eq!(
            refused(prompt.reply(fields(&[too_long]))).message,
            "A Recovery Key has 28 characters. You typed 29."
        );
        let checksum = "QQQQ-QQQQ-QQQQ-QQQQ-QQQQ-QQQQ-QQQQ";
        let refusal = refused(prompt.reply(fields(&[checksum])));
        assert_eq!(
            refusal.message,
            "Something in this key is wrong. Check each group against your paper."
        );

        for value in [typed, bad_symbol, too_long, checksum] {
            let message = refused(prompt.reply(fields(&[value]))).message;
            assert!(!message.contains('Q'), "{message}");
            assert!(!problem_text(&message).contains('Q'));
        }

        let fake_key = vault_format::RecoveryKey::generate().unwrap();
        assert!(matches!(
            prompt.reply(fields(&[&fake_key.expose_display().to_lowercase()])),
            Ok(Reply::Credential(_))
        ));
        // Only a Recovery Key box parses: a password is never judged as one.
        assert!(matches!(
            credential(false).reply(fields(&["fake-password"])),
            Ok(Reply::Credential(_))
        ));
    }

    #[test]
    fn recovery_progress_and_mirror_show_only_what_they_should() {
        assert_eq!(recovery_progress(""), "0 of 28 characters");
        assert_eq!(recovery_progress("ab-cd ef"), "6 of 28 characters");
        assert_eq!(recovery_progress(&"Q".repeat(40)), "40 of 28 characters");

        assert_eq!(recovery_mirror("").as_str(), "");
        assert_eq!(recovery_mirror("abcd").as_str(), "ABCD");
        assert_eq!(recovery_mirror("abcd e").as_str(), "ABCD-E");
        // What the app reads: look-alikes as their symbols, anything else hidden.
        assert_eq!(recovery_mirror("oilu").as_str(), "011?");
        assert_eq!(recovery_mirror("a!é漢").as_str(), "A???");
        let capped = recovery_mirror(&"Q".repeat(40));
        assert_eq!(capped.as_str(), "QQQQ-QQQQ-QQQQ-QQQQ-QQQQ-QQQQ-QQQQ");
        assert!(capped.capacity() >= capped.len());
    }

    #[test]
    fn input_shape_and_size_are_bounded() {
        let prompt = credential(false);
        assert!(prompt.reply(Vec::new()).is_err());
        assert_eq!(
            refused(prompt.reply(fields(&[&"x".repeat(4097)]))).message,
            "That is too long. Use fewer characters."
        );
    }

    #[test]
    fn readouts_and_touch_id_never_submit_and_their_text_says_so() {
        assert_eq!(
            refused(readout(RevealMode::Timed).reply(Vec::new())).message,
            "Press Close to finish."
        );
        assert_eq!(
            refused(biometric().reply(Vec::new())).message,
            "Use Touch ID, or choose Cancel."
        );
        assert_eq!(problem_text("Fake."), "Problem: Fake.");
    }

    #[test]
    fn feedback_names_the_key_this_computer_has() {
        let key = if cfg!(windows) { "Enter" } else { "Return" };
        assert_eq!(
            Request::Entry(kit(false)).feedback_text(),
            format!(
                "You can see what you type here. Press {key} to finish. Press Esc twice to cancel."
            )
        );
        assert_eq!(
            Request::Entry(new_password(false)).feedback_text(),
            format!("{key} moves to the next box, then finishes. Esc cancels.")
        );
        assert_eq!(
            Request::Entry(credential(false)).feedback_text(),
            format!(
                "Press {key} to continue, or Esc to cancel. Switching to another app clears this box."
            )
        );
        for mode in [RevealMode::Held, RevealMode::Timed] {
            assert_eq!(
                readout(mode).feedback_text(),
                "Press Esc or Close when you finish."
            );
        }
        assert_eq!(
            biometric().feedback_text(),
            "Touch the sensor now. Switching to another app or pressing Escape cancels."
        );
    }

    #[test]
    fn time_row_says_what_ends_and_what_that_means() {
        let row = |request: &Request, time: TimeLeft| request.time_row_text(&time);
        let password = Request::Entry(credential(false));
        assert_eq!(row(&password, TimeLeft::Busy), None);
        assert_eq!(row(&password, TimeLeft::Quiet), Some(String::new()));
        assert_eq!(
            row(&password, left(1, TimeLimit::Panel)).unwrap(),
            "This box closes in 1 second. You would need to type again."
        );
        assert_eq!(
            row(&password, left(9, TimeLimit::AutoLock)).unwrap(),
            "The app locks in 9 seconds if you stop typing."
        );
        assert_eq!(
            row(
                &Request::Entry(new_password(true)),
                left(9, TimeLimit::Panel)
            )
            .unwrap(),
            "This box closes in 9 seconds. You would need to type again."
        );

        let setup = Request::Entry(kit(false));
        assert_eq!(
            row(&setup, left(5, TimeLimit::Panel)).unwrap(),
            "This step ends in 5 seconds. Then no Vault is made and this key is never used."
        );
        assert_eq!(
            row(&setup, left(5, TimeLimit::AutoLock)).unwrap(),
            "The app locks in 5 seconds. Typing in the box above gives you more time. If it locks, no Vault is made and this key is never used."
        );
        let rotation = Request::Entry(kit(true));
        for limit in [TimeLimit::Panel, TimeLimit::AutoLock] {
            let text = row(&rotation, left(5, limit)).unwrap();
            assert!(text.contains("your old password and old Recovery Key still work"));
            assert!(!text.contains("stops working"), "{text}");
        }
        assert!(
            row(&rotation, left(5, TimeLimit::AutoLock))
                .unwrap()
                .contains("more time. If it locks, nothing changes:")
        );
        assert_eq!(
            row(&setup, TimeLeft::KitCalm { idle_minutes: 5 }).unwrap(),
            "If you don't type for 5 minutes, the app locks. This step ends after 10 minutes in all."
        );
        assert_eq!(
            row(&setup, TimeLeft::KitCalm { idle_minutes: 1 }).unwrap(),
            "If you don't type for 1 minute, the app locks. This step ends after 10 minutes in all."
        );
        for idle_minutes in [10, 60] {
            assert_eq!(
                row(&setup, TimeLeft::KitCalm { idle_minutes }).unwrap(),
                "This step ends after 10 minutes in all."
            );
        }
        assert_eq!(
            row(&password, TimeLeft::KitCalm { idle_minutes: 5 }),
            Some(String::new())
        );

        let held = readout(RevealMode::Held);
        for limit in [TimeLimit::Panel, TimeLimit::AutoLock] {
            let text = row(&held, left(30, limit)).unwrap();
            assert_eq!(
                text,
                "This screen closes in 30 seconds. You will need your Master Password to show it again."
            );
            assert!(!text.contains("typ") && !text.contains("key") && !text.contains("stay"));
        }
        assert_eq!(
            row(&readout(RevealMode::Timed), left(30, TimeLimit::Panel)),
            Some(String::new())
        );
        assert_eq!(
            row(&biometric(), left(30, TimeLimit::Panel)),
            Some(String::new())
        );
    }

    #[test]
    fn timed_readout_counts_read_correctly() {
        for (seconds, time) in [(1, "1 second"), (2, "2 seconds"), (20, "20 seconds")] {
            for (resets, rest) in [
                (3, "You can reset the timer 3 more times."),
                (2, "You can reset the timer 2 more times."),
                (1, "You can reset the timer 1 more time."),
                (
                    0,
                    "You can't reset the timer again. Enter your Master Password to show it again.",
                ),
            ] {
                assert_eq!(
                    timed_readout_text(seconds, resets),
                    format!("Hides in {time}. {rest}")
                );
            }
        }
    }

    #[test]
    fn the_kit_cancel_asks_once_and_a_bounce_is_not_a_second_press() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);

        let mut arm = CancelArm::default();
        assert!(!arm.expired(at(0)));
        assert_eq!(arm.press(at(0), true), ArmDecision::Arm);
        assert_eq!(arm.press(at(299), true), ArmDecision::Ignore);
        assert_eq!(arm.press(at(300), true), ArmDecision::Cancel);

        // A double-click on Cancel is one press.
        let mut arm = CancelArm::default();
        assert_eq!(arm.press(at(0), false), ArmDecision::Arm);
        assert_eq!(arm.press(at(499), false), ArmDecision::Ignore);
        assert_eq!(arm.press(at(500), false), ArmDecision::Cancel);

        // Ten seconds later it asks again.
        let mut arm = CancelArm::default();
        assert_eq!(arm.press(at(0), true), ArmDecision::Arm);
        assert!(!arm.expired(at(10_000)));
        assert_eq!(arm.press(at(10_000), true), ArmDecision::Cancel);
        let mut arm = CancelArm::default();
        assert_eq!(arm.press(at(0), true), ArmDecision::Arm);
        assert!(arm.expired(at(10_001)));
        assert_eq!(arm.press(at(10_001), true), ArmDecision::Arm);
        assert_eq!(arm.press(at(10_400), true), ArmDecision::Cancel);

        // Typing disarms.
        let mut arm = CancelArm::default();
        assert_eq!(arm.press(at(0), true), ArmDecision::Arm);
        arm.disarm();
        assert!(!arm.expired(at(20_000)));
        assert_eq!(arm.press(at(400), true), ArmDecision::Arm);
    }

    fn settled(live: &mut LiveCheck) -> bool {
        for _ in 0..1_000 {
            if live.poll() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn live_line_waits_for_a_pause_and_counts_like_the_rule() {
        let start = Instant::now();
        let mut live = LiveCheck::default();
        assert!(!live.due(start));
        live.edited(start);
        assert!(!live.due(start + Duration::from_millis(299)));
        assert!(live.due(start + Duration::from_millis(300)));
        // A held key edits again and again: the line waits for the release, then reads the boxes.
        live.edited(start + Duration::from_millis(200));
        assert!(!live.due(start + Duration::from_millis(300)));
        assert!(live.due(start + Duration::from_millis(500)));
        // Decomposed letters count once, like the Vault's NFC form.
        let fake = "e\u{301}e\u{301}e\u{301}";
        assert_eq!(policy::char_count(fake), 3);
        assert_eq!(live.line(fake, ""), "3 of 15 characters");
        assert!(!live.due(start + Duration::from_secs(1)));
        assert_eq!(
            live.line(fake, fake),
            "3 of 15 characters. The two passwords match."
        );
        assert_eq!(
            live.line(fake, "fake"),
            "3 of 15 characters. The two passwords don't match yet."
        );
    }

    #[test]
    fn live_line_reports_strength_only_for_the_latest_text() {
        let start = Instant::now();
        let strong = policy::suggest_passphrase();
        let mut live = LiveCheck::default();
        live.edited(start);
        assert_eq!(live.line(&strong, ""), "Checking…");
        assert!(settled(&mut live));
        assert_eq!(live.line(&strong, ""), "Strong enough.");
        assert_eq!(
            live.line(&strong, &strong),
            "Strong enough. The two passwords match."
        );

        // An edit to New forgets the answer: nothing is reused for other text.
        let weak = "fake fake fake fake fake";
        live.edited(start);
        assert_eq!(
            live.line(weak, "other"),
            "Checking… The two passwords don't match yet."
        );
        assert!(settled(&mut live));
        assert_eq!(
            live.line(weak, ""),
            "Not strong enough yet. Add another unusual word."
        );
    }

    #[test]
    fn an_edit_to_confirm_keeps_the_answer_about_new() {
        let start = Instant::now();
        let strong = policy::suggest_passphrase();
        let mut live = LiveCheck::default();
        // Change password: Current, New, Confirm.
        live.box_edited(start, 3, Some(1));
        assert_eq!(live.line(&strong, ""), "Checking…");
        assert!(settled(&mut live));
        let generation = live.generation;

        live.box_edited(start, 3, Some(2));
        assert!(live.due(start + Duration::from_millis(300)));
        assert_eq!(
            live.line(&strong, "other"),
            "Strong enough. The two passwords don't match yet."
        );
        assert_eq!(live.generation, generation);
        assert!(!live.running);

        // A worker running for the same New still delivers.
        live.strength = None;
        live.running = true;
        live.box_edited(start, 3, Some(2));
        *live.slot.lock().unwrap() = Some((generation, true));
        assert!(live.poll());
        assert_eq!(live.strength, Some(true));

        // New, or a box that can't be told apart, forgets it.
        for index in [Some(1), None] {
            live.strength = Some(true);
            live.box_edited(start, 3, index);
            assert_eq!(live.strength, None);
            assert_eq!(
                live.line(&strong, "other"),
                "Checking… The two passwords don't match yet."
            );
            assert!(settled(&mut live));
        }
    }

    #[test]
    fn a_stale_strength_answer_is_dropped() {
        let start = Instant::now();
        let mut live = LiveCheck::default();
        live.edited(start);
        let old = live.generation;
        // The answer for older text arrives after another edit.
        live.running = true;
        live.edited(start);
        *live.slot.lock().unwrap() = Some((old, true));
        assert!(!live.poll());
        assert_eq!(live.strength, None);
        assert!(!live.running);

        // Text that had to wait for the worker gets its turn.
        live.running = true;
        let strong = policy::suggest_passphrase();
        assert_eq!(live.line(&strong, ""), "Checking…");
        assert!(live.waiting);
        *live.slot.lock().unwrap() = Some((old, true));
        assert!(live.poll());
        assert_eq!(live.strength, None);
        assert_eq!(live.line(&strong, ""), "Checking…");
        assert!(settled(&mut live));
        assert_eq!(live.line(&strong, ""), "Strong enough.");
    }

    #[test]
    fn live_line_skips_all_checks_on_oversize_text() {
        let oversize = "x".repeat(4097);
        let mut live = LiveCheck::default();
        live.edited(Instant::now());
        let too_long = "That is too long. Use fewer characters.";
        assert_eq!(live.line(&oversize, ""), too_long);
        assert_eq!(live.line("fake", &oversize), too_long);
        assert!(!live.running);
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
