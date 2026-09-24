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
//! * Always lock on `Sleep`, `ScreenLocked`, `UserSwitched`, `Quit`; also on `FocusLost` when
//!   `lock_on_app_switch` is on.
//! * Wrong credential delays: attempts 1-3 have no delay; after the 3rd consecutive failure the
//!   next attempt waits 1 s, then 2 s, 5 s, 10 s, and 30 s for every attempt after that.
//!   A success resets the count. Nothing is ever erased.
//! * Reveal: at most one shown Hidden Field. `Timed` expires after 30 s, `Accessible` after 20 s,
//!   `Held` lasts until `end_reveal`. Any shown value is hidden on: lock, opening another Note,
//!   `FocusLost`, or the Sharing Guard going up. While the guard is up or the app is Locked, a new
//!   reveal is refused.
//! * Clipboard: a copy is cleared after 30 s, on lock, and on quit, but only if the clipboard's
//!   change counter still equals the value right after our copy.
//! * Backup reminder: due when the oldest change not yet in a Backup is 7+ days old.

#![forbid(unsafe_code)]

/// Milliseconds from any fixed origin (monotonic).
pub type Millis = u64;

pub const TIMED_REVEAL_MS: Millis = 30_000;
pub const ACCESSIBLE_REVEAL_MS: Millis = 20_000;
pub const CLIPBOARD_CLEAR_MS: Millis = 30_000;
pub const BACKUP_REMINDER_MS: Millis = 7 * 24 * 60 * 60 * 1000;
/// Idle choices offered in Settings; default 5.
pub const IDLE_MINUTE_CHOICES: [u32; 7] = [1, 2, 5, 10, 15, 30, 60];
pub const DEFAULT_IDLE_MINUTES: u32 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LockSettings {
    /// One of [`IDLE_MINUTE_CHOICES`].
    pub idle_minutes: u32,
    pub lock_on_app_switch: bool,
}

impl Default for LockSettings {
    fn default() -> Self {
        LockSettings {
            idle_minutes: DEFAULT_IDLE_MINUTES,
            lock_on_app_switch: false,
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
    /// `None` for `Held`.
    pub expires_at: Option<Millis>,
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
    /// `Some(last_activity)` while Unlocked.
    last_activity: Option<Millis>,
    failures: u32,
    last_failure_at: Millis,
    reveal: Option<Reveal>,
    sharing_guard_up: bool,
    clipboard: Option<PendingClear>,
}

/// Keeps `idle_minutes` within the offered choices by snapping to the nearest one
/// (the shorter one on a tie, which is the safer side).
fn sanitize(settings: LockSettings) -> LockSettings {
    let wanted = settings.idle_minutes;
    let idle_minutes = IDLE_MINUTE_CHOICES
        .iter()
        .copied()
        .min_by_key(|choice| choice.abs_diff(wanted))
        .unwrap_or(DEFAULT_IDLE_MINUTES);
    LockSettings {
        idle_minutes,
        ..settings
    }
}

impl Session {
    /// Starts Locked.
    pub fn new(settings: LockSettings) -> Session {
        Session {
            settings: sanitize(settings),
            last_activity: None,
            failures: 0,
            last_failure_at: 0,
            reveal: None,
            sharing_guard_up: false,
            clipboard: None,
        }
    }

    pub fn settings(&self) -> LockSettings {
        self.settings
    }
    pub fn set_settings(&mut self, settings: LockSettings) {
        self.settings = sanitize(settings);
    }

    pub fn is_unlocked(&self) -> bool {
        self.last_activity.is_some()
    }

    /// Credential accepted: become Unlocked, reset failures, start the idle timer.
    pub fn unlocked(&mut self, now: Millis) {
        self.last_activity = Some(now);
        self.failures = 0;
    }

    /// Owner pressed Lock (or an Effect::Lock is being carried out). Returns HideReveal /
    /// ClearClipboardIfUnchanged as needed, plus Lock.
    pub fn lock(&mut self) -> Vec<Effect> {
        let mut effects = self.end_reveal();
        if let Some(pending) = self.clipboard.take() {
            effects.push(Effect::ClearClipboardIfUnchanged(pending.change_count));
        }
        effects.extend(self.last_activity.take().map(|_| Effect::Lock));
        effects
    }

    /// Owner interacted with the app. Activity at or after the lock deadline does not revive the
    /// session: the Auto-lock was already due, and the next `tick` carries it out.
    pub fn activity(&mut self, now: Millis) {
        if !self.auto_lock_due(now) {
            self.last_activity = self.last_activity.map(|last| last.max(now));
        }
    }

    /// Call about once a second. Returns due effects (idle lock, reveal expiry, clipboard clear).
    pub fn tick(&mut self, now: Millis) -> Vec<Effect> {
        if self.auto_lock_due(now) {
            return self.lock();
        }
        let mut effects = Vec::new();
        if self
            .reveal
            .take_if(|reveal| reveal.expires_at.is_some_and(|at| now >= at))
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
            OsEvent::FocusLost if self.settings.lock_on_app_switch => self.lock(),
            OsEvent::FocusLost => self.end_reveal(),
            OsEvent::FocusGained => Vec::new(),
        }
    }

    /// When the app will auto-lock if nothing else happens (for the countdown). `None` if Locked.
    pub fn lock_deadline(&self) -> Option<Millis> {
        let idle_ms = Millis::from(self.settings.idle_minutes) * 60_000;
        self.last_activity.map(|last| last.saturating_add(idle_ms))
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
        let expires_at = match mode {
            RevealMode::Timed => Some(now.saturating_add(TIMED_REVEAL_MS)),
            RevealMode::Accessible => Some(now.saturating_add(ACCESSIBLE_REVEAL_MS)),
            RevealMode::Held => None,
        };
        self.reveal = Some(Reveal {
            note_id: note_id.to_owned(),
            field: field.to_owned(),
            mode,
            expires_at,
        });
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
    pub fn copied(&mut self, change_count: u64, now: Millis) {
        self.clipboard = Some(PendingClear {
            change_count,
            due_at: now.saturating_add(CLIPBOARD_CLEAR_MS),
        });
    }
    /// When the pending clipboard clear is due (for the countdown). `None` if nothing pending.
    pub fn clipboard_clear_at(&self) -> Option<Millis> {
        self.clipboard.as_ref().map(|pending| pending.due_at)
    }

    fn auto_lock_due(&self, now: Millis) -> bool {
        self.lock_deadline().is_some_and(|deadline| now >= deadline)
    }
}

/// Backup reminder rule. `oldest_unbacked_change` is when the first change after the last
/// Backup happened (`None` if everything is backed up). Wall-clock milliseconds are fine here.
pub fn backup_reminder_due(oldest_unbacked_change: Option<Millis>, now: Millis) -> bool {
    oldest_unbacked_change
        .is_some_and(|changed_at| now.saturating_sub(changed_at) >= BACKUP_REMINDER_MS)
}
