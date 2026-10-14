//! Session rules for an open app, as a pure state machine. The app feeds it events and a
//! monotonic clock (`now` in milliseconds) and carries out the [`Effect`]s it returns: dropping
//! keys, hiding a shown value, clearing the clipboard. No keys live here. This crate decides
//! *when*; the app holds *what*.
//!
//! Vocabulary: CONTEXT.md (**Locked**/**Unlocked**, **Auto-lock**, **Hidden Field**,
//! **Wallet Kinds**, **Sharing Guard**).
//!
//! Rules (each one has tests):
//! * Auto-lock: Unlocked -> Locked when `now - last_activity >= idle_minutes`. Activity is any
//!   owner interaction the screens report.
//! * Activity anywhere (`count_activity_anywhere`, off unless the owner turns it on): hardware
//!   input elsewhere on the computer, which the app reads from the OS, keeps Auto-lock away
//!   too. The idle time then runs from the last activity of either kind, but the app locks no
//!   later than 60 minutes after the last activity inside its own window. Activity elsewhere
//!   never moves that limit, and no activity report of either kind revives a session whose
//!   lock is already due.
//! * Always lock on `Sleep`, `ScreenLocked`, `UserSwitched`, `Quit`; also on `FocusLost` when
//!   `lock_on_app_switch` is on.
//! * Wrong credential delays: attempts 1-3 have no delay; after the 3rd consecutive failure the
//!   next attempt waits 1 s, then 2 s, 5 s, 10 s, and 30 s for every attempt after that.
//!   A success resets the count. Nothing is ever erased. The app keeps the count across restarts
//!   ([`Session::restore_failures`]).
//! * Reveal: at most one shown Hidden Field. `Timed` expires after 30 s, `Accessible` after 20 s,
//!   `Held` lasts until `end_reveal`, or 2 minutes if the release never arrives. Any shown value
//!   is hidden on: lock, opening another Note, `FocusLost`, or the Sharing Guard going up. While
//!   the guard is up or the app is Locked, a new reveal is refused. So is a 9th different field
//!   within a minute: the screens never ask for that, so something else is asking.
//! * More time: a value that is still shown can be given its time again, counted from the
//!   moment of asking. `Timed` gets another 30 s as often as asked. `Accessible` gets another
//!   20 s at most three times; after that the value has to be shown again, which takes the
//!   Master Password. `Held` has no timer and is refused. A reveal that has ended is never
//!   brought back.
//! * Clipboard: a copy is cleared after 30 s (or 60 s if the owner chose that; a Wallet Kind
//!   value always after 30 s), on lock, and on quit, but only if the clipboard's change counter
//!   still equals the value right after our copy. A lock caused by switching apps
//!   (`lock_on_app_switch`) leaves that timer running, so the copy can still be pasted.
//! * Backup reminder: due when the oldest change not yet in a Backup is 7+ days old.

#![forbid(unsafe_code)]

/// Milliseconds from any fixed origin (monotonic).
pub type Millis = u64;

pub const TIMED_REVEAL_MS: Millis = 30_000;
pub const ACCESSIBLE_REVEAL_MS: Millis = 20_000;
/// Backstop for a `Held` reveal whose release the screens never saw (e.g. a lost key-up).
pub const HELD_REVEAL_MAX_MS: Millis = 120_000;
/// At most [`MAX_FIELDS_PER_WINDOW`] different Hidden Fields may be shown within this long.
pub const REVEAL_WINDOW_MS: Millis = 60_000;
pub const MAX_FIELDS_PER_WINDOW: usize = 8;
/// How many times one `Accessible` reveal can be given another 20 s.
pub const MAX_ACCESSIBLE_EXTENSIONS: u32 = 3;
/// How long a copy stays on the clipboard by default, and always for a Wallet Kind value.
pub const CLIPBOARD_CLEAR_MS: Millis = 30_000;
/// Clipboard clear choices offered in Settings, in seconds; default 30.
pub const CLIPBOARD_CLEAR_SECOND_CHOICES: [u32; 2] = [30, 60];
pub const DEFAULT_CLIPBOARD_CLEAR_SECONDS: u32 = 30;
pub const BACKUP_REMINDER_MS: Millis = 7 * 24 * 60 * 60 * 1000;
/// Idle choices offered in Settings; default 5.
pub const IDLE_MINUTE_CHOICES: [u32; 7] = [1, 2, 5, 10, 15, 30, 60];
pub const DEFAULT_IDLE_MINUTES: u32 = 5;
/// With `count_activity_anywhere` on, the app still locks this long after the last activity
/// inside its own window, whatever is happening elsewhere on the computer.
pub const ACTIVITY_ANYWHERE_CAP_MS: Millis = 60 * 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockSettings {
    /// One of [`IDLE_MINUTE_CHOICES`].
    pub idle_minutes: u32,
    pub lock_on_app_switch: bool,
    /// One of [`CLIPBOARD_CLEAR_SECOND_CHOICES`]. Not used for a Wallet Kind value.
    pub clipboard_clear_seconds: u32,
    /// Hardware input elsewhere on the computer also keeps Auto-lock away, up to
    /// [`ACTIVITY_ANYWHERE_CAP_MS`] after the last activity inside the app's window.
    pub count_activity_anywhere: bool,
}

impl Default for LockSettings {
    fn default() -> Self {
        LockSettings {
            idle_minutes: DEFAULT_IDLE_MINUTES,
            lock_on_app_switch: false,
            clipboard_clear_seconds: DEFAULT_CLIPBOARD_CLEAR_SECONDS,
            count_activity_anywhere: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsEvent {
    Sleep,
    ScreenLocked,
    UserSwitched,
    Quit,
    FocusLost,
    FocusGained,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevealMode {
    /// Ordinary Kinds: 30 s.
    Timed,
    /// Wallet Kinds: while the Show button / Space is held.
    Held,
    /// Wallet Kinds, keyboard/VoiceOver alternative: 20 s.
    Accessible,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reveal {
    pub note_id: String,
    pub field: String,
    pub mode: RevealMode,
    /// `None` for `Held` (which still ends after [`HELD_REVEAL_MAX_MS`]).
    pub expires_at: Option<Millis>,
    /// How many times this reveal was given more time ([`Session::extend_reveal`]). A value
    /// that is shown again starts from 0.
    pub extensions: u32,
}

impl Reveal {
    /// Whether [`Session::extend_reveal`] would still give this reveal more time: always for
    /// `Timed`, up to [`MAX_ACCESSIBLE_EXTENSIONS`] times for `Accessible`, never for `Held`.
    pub fn can_extend(&self) -> bool {
        match self.mode {
            RevealMode::Timed => true,
            RevealMode::Accessible => self.extensions < MAX_ACCESSIBLE_EXTENSIONS,
            RevealMode::Held => false,
        }
    }

    /// How many more times an `Accessible` reveal can be given more time. `None` for the other
    /// modes: `Timed` has no limit and `Held` can't be extended at all.
    pub fn extensions_left(&self) -> Option<u32> {
        matches!(self.mode, RevealMode::Accessible)
            .then(|| MAX_ACCESSIBLE_EXTENSIONS.saturating_sub(self.extensions))
    }
}

/// Things the app must do now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Drop the Vault Key and decrypted document, show the lock screen.
    Lock,
    /// Stop showing the currently shown Hidden Field.
    HideReveal,
    /// Clear the clipboard if its change counter still equals this value.
    ClearClipboardIfUnchanged(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RevealRefused {
    Locked,
    SharingGuardUp,
    /// Too many different Hidden Fields in a short time; try again in this many milliseconds.
    TooMany(Millis),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtendRefused {
    /// Nothing is shown: no value was, or it has ended (or is due to end right now).
    NothingShown,
    /// A `Held` reveal lasts while the button is held; it has no timer to extend.
    Held,
    /// This `Accessible` reveal already had [`MAX_ACCESSIBLE_EXTENSIONS`] extensions. Showing
    /// the value again takes the Master Password.
    LimitReached,
}

/// Delays after the 4th, 5th, 6th, 7th and every later consecutive failure.
const FAILURE_DELAYS_MS: [Millis; 5] = [1_000, 2_000, 5_000, 10_000, 30_000];
/// Consecutive failures allowed without any delay.
const FREE_ATTEMPTS: u32 = 3;

struct PendingClear {
    change_count: u64,
    due_at: Millis,
}

pub struct Session {
    settings: LockSettings,
    /// `Some(last_activity)` while Unlocked: the last activity inside the app's own window.
    last_activity: Option<Millis>,
    /// The last hardware input seen elsewhere on the computer since the last unlock. Only ever
    /// set while `count_activity_anywhere` is on, and never mixed into `last_activity`.
    last_elsewhere: Option<Millis>,
    failures: u32,
    last_failure_at: Millis,
    reveal: Option<Reveal>,
    /// When a `Held` reveal is hidden even without `end_reveal`.
    held_until: Option<Millis>,
    /// (first shown at, note id, field) for each field shown within [`REVEAL_WINDOW_MS`]. Kept
    /// across locks, so locking and unlocking again doesn't start a fresh allowance.
    recently_shown: Vec<(Millis, String, String)>,
    sharing_guard_up: bool,
    clipboard: Option<PendingClear>,
}

/// Keeps `idle_minutes` within the offered choices by snapping to the nearest one
/// (the shorter one on a tie, which is the safer side), and `clipboard_clear_seconds` within
/// its two: anything else becomes the default, which is the shorter one.
fn sanitize(settings: LockSettings) -> LockSettings {
    let wanted = settings.idle_minutes;
    let idle_minutes = IDLE_MINUTE_CHOICES
        .iter()
        .copied()
        .min_by_key(|choice| choice.abs_diff(wanted))
        .unwrap_or(DEFAULT_IDLE_MINUTES);
    let clipboard_clear_seconds =
        if CLIPBOARD_CLEAR_SECOND_CHOICES.contains(&settings.clipboard_clear_seconds) {
            settings.clipboard_clear_seconds
        } else {
            DEFAULT_CLIPBOARD_CLEAR_SECONDS
        };
    LockSettings {
        idle_minutes,
        clipboard_clear_seconds,
        ..settings
    }
}

impl Session {
    /// Starts Locked.
    pub fn new(settings: LockSettings) -> Session {
        Session {
            settings: sanitize(settings),
            last_activity: None,
            last_elsewhere: None,
            failures: 0,
            last_failure_at: 0,
            reveal: None,
            held_until: None,
            recently_shown: Vec::new(),
            sharing_guard_up: false,
            clipboard: None,
        }
    }

    pub fn settings(&self) -> LockSettings {
        self.settings
    }
    pub fn set_settings(&mut self, settings: LockSettings) {
        self.settings = sanitize(settings);
        if !self.settings.count_activity_anywhere {
            self.last_elsewhere = None;
        }
    }

    pub fn is_unlocked(&self) -> bool {
        self.last_activity.is_some()
    }

    /// Credential accepted: become Unlocked, reset failures, start the idle timer.
    pub fn unlocked(&mut self, now: Millis) {
        self.last_activity = Some(now);
        self.last_elsewhere = None;
        self.failures = 0;
    }

    /// Owner pressed Lock (or an Effect::Lock is being carried out). Returns HideReveal /
    /// ClearClipboardIfUnchanged as needed, plus Lock.
    pub fn lock(&mut self) -> Vec<Effect> {
        let mut effects = self.end_reveal();
        if let Some(pending) = self.clipboard.take() {
            effects.push(Effect::ClearClipboardIfUnchanged(pending.change_count));
        }
        effects.extend(self.become_locked());
        effects
    }

    /// Owner interacted with the app. Activity at or after the lock deadline does not revive the
    /// session: the Auto-lock was already due, and the next `tick` carries it out.
    pub fn activity(&mut self, now: Millis) {
        if !self.auto_lock_due(now) {
            self.last_activity = self.last_activity.map(|last| last.max(now));
        }
    }

    /// Hardware input was seen somewhere on this computer at `at` (the app asks the OS; the
    /// screens have no way to report this). Counts only while `count_activity_anywhere` is on
    /// and the app is Unlocked.
    ///
    /// It is kept apart from the activity inside the window: it can move the idle time, never
    /// the [`ACTIVITY_ANYWHERE_CAP_MS`] limit, which is counted from the last activity inside
    /// the window alone. Like [`Session::activity`] it does not revive a session whose lock is
    /// already due at `now`, and an `at` later than `now` counts as `now`.
    pub fn activity_elsewhere(&mut self, at: Millis, now: Millis) {
        if !self.settings.count_activity_anywhere || !self.is_unlocked() || self.auto_lock_due(now)
        {
            return;
        }
        let at = at.min(now);
        self.last_elsewhere = Some(self.last_elsewhere.map_or(at, |last| last.max(at)));
    }

    /// Call about once a second. Returns due effects (idle lock, reveal expiry, clipboard clear).
    pub fn tick(&mut self, now: Millis) -> Vec<Effect> {
        if self.auto_lock_due(now) {
            return self.lock();
        }
        let mut effects = Vec::new();
        let held_until = self.held_until;
        if self
            .reveal
            .take_if(|reveal| reveal.expires_at.or(held_until).is_some_and(|at| now >= at))
            .is_some()
        {
            effects.push(Effect::HideReveal);
        }
        if let Some(pending) = self.clipboard.take_if(|pending| now >= pending.due_at) {
            effects.push(Effect::ClearClipboardIfUnchanged(pending.change_count));
        }
        effects
    }

    pub fn os_event(&mut self, event: OsEvent, _now: Millis) -> Vec<Effect> {
        match event {
            OsEvent::Sleep | OsEvent::ScreenLocked | OsEvent::UserSwitched | OsEvent::Quit => {
                self.lock()
            }
            // Lock, but leave a pending clipboard clear on its timer (`tick` carries it out
            // while Locked too): the owner most likely switched apps to paste what they copied.
            OsEvent::FocusLost if self.settings.lock_on_app_switch => {
                let mut effects = self.end_reveal();
                effects.extend(self.become_locked());
                effects
            }
            OsEvent::FocusLost => self.end_reveal(),
            OsEvent::FocusGained => Vec::new(),
        }
    }

    /// When the app will auto-lock if nothing else happens (for the countdown). `None` if Locked.
    ///
    /// The idle time after the last activity inside the window. With `count_activity_anywhere`
    /// on, activity elsewhere counts for the idle time too, but the deadline is never later than
    /// [`ACTIVITY_ANYWHERE_CAP_MS`] after the last activity inside the window.
    pub fn lock_deadline(&self) -> Option<Millis> {
        let idle_ms = Millis::from(self.settings.idle_minutes) * 60_000;
        let inside = self.last_activity?;
        if !self.settings.count_activity_anywhere {
            return Some(inside.saturating_add(idle_ms));
        }
        let latest = self.last_elsewhere.map_or(inside, |at| at.max(inside));
        let cap = inside.saturating_add(ACTIVITY_ANYWHERE_CAP_MS);
        Some(latest.saturating_add(idle_ms).min(cap))
    }

    /// `Err(wait_ms)` if the owner must wait before trying another credential.
    pub fn attempt_allowed(&self, now: Millis) -> Result<(), Millis> {
        let Some(extra) = self.failures.checked_sub(FREE_ATTEMPTS) else {
            return Ok(());
        };
        let index = usize::try_from(extra)
            .unwrap_or(usize::MAX)
            .min(FAILURE_DELAYS_MS.len() - 1);
        let ready_at = self
            .last_failure_at
            .saturating_add(FAILURE_DELAYS_MS[index]);
        if now >= ready_at {
            Ok(())
        } else {
            Err(ready_at - now)
        }
    }
    pub fn failed_attempt(&mut self, now: Millis) {
        self.failures = self.failures.saturating_add(1);
        self.last_failure_at = now;
    }
    /// At startup: carry on from the failures counted before the app was closed, with the delay
    /// they earned starting now. Without this, quitting and reopening buys three free attempts.
    pub fn restore_failures(&mut self, failures: u32, now: Millis) {
        self.failures = failures;
        self.last_failure_at = now;
    }
    pub fn failed_attempts(&self) -> u32 {
        self.failures
    }
    /// True after 3+ consecutive failures ("Forgot it? Use your Recovery Kit").
    pub fn show_recovery_hint(&self) -> bool {
        self.failures >= FREE_ATTEMPTS
    }

    /// Shows a Hidden Field. Refused while Locked (including once the Auto-lock is due at `now`,
    /// even if the tick that carries it out has not run yet) or while the Sharing Guard is up.
    ///
    /// There is at most one shown Hidden Field: on `Ok`, this reveal *replaces* any earlier one,
    /// so the app shows only [`Session::current_reveal`] and must stop showing whatever it showed
    /// before. No separate `HideReveal` is returned for the replaced one.
    pub fn start_reveal(
        &mut self,
        note_id: &str,
        field: &str,
        mode: RevealMode,
        now: Millis,
    ) -> Result<(), RevealRefused> {
        if !self.is_unlocked() || self.auto_lock_due(now) {
            return Err(RevealRefused::Locked);
        }
        if self.sharing_guard_up {
            return Err(RevealRefused::SharingGuardUp);
        }
        self.recently_shown
            .retain(|(at, ..)| now < at.saturating_add(REVEAL_WINDOW_MS));
        let shown_before = self
            .recently_shown
            .iter()
            .any(|(_, n, f)| n == note_id && f == field);
        if !shown_before {
            if self.recently_shown.len() >= MAX_FIELDS_PER_WINDOW {
                let oldest = self.recently_shown.iter().map(|(at, ..)| *at).min();
                let free_at = oldest.unwrap_or(now).saturating_add(REVEAL_WINDOW_MS);
                return Err(RevealRefused::TooMany(free_at.saturating_sub(now).max(1)));
            }
            self.recently_shown
                .push((now, note_id.to_owned(), field.to_owned()));
        }
        let expires_at = match mode {
            RevealMode::Timed => Some(now.saturating_add(TIMED_REVEAL_MS)),
            RevealMode::Accessible => Some(now.saturating_add(ACCESSIBLE_REVEAL_MS)),
            RevealMode::Held => None,
        };
        self.held_until =
            matches!(mode, RevealMode::Held).then(|| now.saturating_add(HELD_REVEAL_MAX_MS));
        self.reveal = Some(Reveal {
            note_id: note_id.to_owned(),
            field: field.to_owned(),
            mode,
            expires_at,
            extensions: 0,
        });
        Ok(())
    }

    /// The owner asked for more time for the value that is shown. On `Ok` its time starts again
    /// from `now`: 30 s for `Timed`, 20 s for `Accessible`.
    ///
    /// Only a reveal that is still shown at `now` is extended. One that has ended, in any of the
    /// ways a reveal ends, is not brought back, and neither is one whose time is up at `now`
    /// although the tick that hides it has not run yet. `Timed` can be extended as often as
    /// asked; `Accessible` [`MAX_ACCESSIBLE_EXTENSIONS`] times, counted in the reveal, so
    /// showing the value again (with the Master Password) starts a new count; `Held` never.
    pub fn extend_reveal(&mut self, now: Millis) -> Result<(), ExtendRefused> {
        if self.auto_lock_due(now) {
            return Err(ExtendRefused::NothingShown);
        }
        let Some(reveal) = self.reveal.as_mut() else {
            return Err(ExtendRefused::NothingShown);
        };
        let length = match reveal.mode {
            RevealMode::Held => return Err(ExtendRefused::Held),
            RevealMode::Timed => TIMED_REVEAL_MS,
            RevealMode::Accessible => ACCESSIBLE_REVEAL_MS,
        };
        let Some(expires_at) = reveal.expires_at.filter(|at| now < *at) else {
            return Err(ExtendRefused::NothingShown);
        };
        if !reveal.can_extend() {
            return Err(ExtendRefused::LimitReached);
        }
        reveal.extensions = reveal.extensions.saturating_add(1);
        // Never earlier than it already was: asking for more time can't take time away.
        reveal.expires_at = Some(expires_at.max(now.saturating_add(length)));
        Ok(())
    }

    /// Held button released (or owner clicked Hide). `HideReveal` if a value was shown.
    pub fn end_reveal(&mut self) -> Vec<Effect> {
        Vec::from_iter(self.reveal.take().map(|_| Effect::HideReveal))
    }
    pub fn current_reveal(&self) -> Option<&Reveal> {
        self.reveal.as_ref()
    }

    /// The owner opened a Note in the detail view.
    pub fn note_opened(&mut self, note_id: &str) -> Vec<Effect> {
        if self
            .reveal
            .as_ref()
            .is_some_and(|reveal| reveal.note_id != note_id)
        {
            self.end_reveal()
        } else {
            Vec::new()
        }
    }

    /// Sharing Guard state from the guard crate. Going up hides any reveal.
    pub fn set_sharing_guard(&mut self, up: bool) -> Vec<Effect> {
        self.sharing_guard_up = up;
        if up { self.end_reveal() } else { Vec::new() }
    }
    pub fn sharing_guard_up(&self) -> bool {
        self.sharing_guard_up
    }

    /// We just wrote to the clipboard; `change_count` is the clipboard's counter after our write.
    /// Cleared after the time the owner chose (`clipboard_clear_seconds`). A Wallet Kind value
    /// is reported with [`Session::copied_wallet_value`] instead.
    pub fn copied(&mut self, change_count: u64, now: Millis) {
        let clear_ms = Millis::from(self.settings.clipboard_clear_seconds) * 1000;
        self.clipboard = Some(PendingClear {
            change_count,
            due_at: now.saturating_add(clear_ms),
        });
    }

    /// We just wrote a Wallet Kind value to the clipboard: cleared after [`CLIPBOARD_CLEAR_MS`],
    /// whatever `clipboard_clear_seconds` says.
    pub fn copied_wallet_value(&mut self, change_count: u64, now: Millis) {
        self.clipboard = Some(PendingClear {
            change_count,
            due_at: now.saturating_add(CLIPBOARD_CLEAR_MS),
        });
    }

    /// Keep a failed clear retryable after its original timer or lock event has fired.
    pub fn retry_clipboard_clear(&mut self, change_count: u64, now: Millis) {
        if self
            .clipboard
            .as_ref()
            .is_none_or(|pending| pending.change_count == change_count)
        {
            self.clipboard = Some(PendingClear {
                change_count,
                due_at: now.saturating_add(250),
            });
        }
    }

    /// When the pending clipboard clear is due (for the countdown). `None` if nothing pending.
    pub fn clipboard_clear_at(&self) -> Option<Millis> {
        self.clipboard.as_ref().map(|pending| pending.due_at)
    }

    fn auto_lock_due(&self, now: Millis) -> bool {
        self.lock_deadline().is_some_and(|deadline| now >= deadline)
    }

    /// Forget when the owner was last active, here and elsewhere. `Lock` if the app was Unlocked.
    fn become_locked(&mut self) -> Option<Effect> {
        self.last_elsewhere = None;
        self.last_activity.take().map(|_| Effect::Lock)
    }
}

/// Backup reminder rule. `oldest_unbacked_change` is when the first change after the last
/// Backup happened (`None` if everything is backed up). Wall-clock milliseconds are fine here.
pub fn backup_reminder_due(oldest_unbacked_change: Option<Millis>, now: Millis) -> bool {
    oldest_unbacked_change
        .is_some_and(|changed_at| now.saturating_sub(changed_at) >= BACKUP_REMINDER_MS)
}
