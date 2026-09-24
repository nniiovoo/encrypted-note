//! Behaviour tests for the session rules, driven by a fake clock (`now` values).

use session::*;

const SEC: Millis = 1_000;
const MIN: Millis = 60 * SEC;
const T0: Millis = 1_000_000;

fn unlocked_session(settings: LockSettings) -> Session {
    let mut s = Session::new(settings);
    s.unlocked(T0);
    s
}

fn default_unlocked() -> Session {
    unlocked_session(LockSettings::default())
}

// ---------- Locked / Unlocked ----------

#[test]
fn new_session_starts_locked_and_activity_does_not_unlock_it() {
    let mut s = Session::new(LockSettings::default());
    s.activity(T0);
    assert!(!s.is_unlocked());
    assert_eq!(s.lock_deadline(), None);
}

#[test]
fn unlocked_makes_session_unlocked_and_starts_idle_timer() {
    let s = default_unlocked();
    assert!(s.is_unlocked());
    assert_eq!(s.lock_deadline(), Some(T0 + 5 * MIN));
}

#[test]
fn owner_lock_returns_lock_and_becomes_locked() {
    let mut s = default_unlocked();
    assert_eq!(s.lock(), vec![Effect::Lock]);
    assert!(!s.is_unlocked());
    assert_eq!(s.lock_deadline(), None);
}

#[test]
fn lock_while_already_locked_with_nothing_pending_does_nothing() {
    let mut s = Session::new(LockSettings::default());
    assert_eq!(s.lock(), vec![]);
}

#[test]
fn lock_hides_reveal_and_clears_clipboard_before_locking() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    s.copied(42, T0);
    assert_eq!(
        s.lock(),
        vec![
            Effect::HideReveal,
            Effect::ClearClipboardIfUnchanged(42),
            Effect::Lock
        ]
    );
    assert_eq!(s.current_reveal(), None);
    assert_eq!(s.clipboard_clear_at(), None);
}

#[test]
fn settings_round_trip() {
    let mut s = Session::new(LockSettings::default());
    assert_eq!(
        s.settings(),
        LockSettings {
            idle_minutes: 5,
            lock_on_app_switch: false
        }
    );
    let new = LockSettings {
        idle_minutes: 15,
        lock_on_app_switch: true,
    };
    s.set_settings(new);
    assert_eq!(s.settings(), new);
}

#[test]
fn idle_minutes_outside_the_choices_snaps_to_nearest_choice() {
    let mut s = Session::new(LockSettings {
        idle_minutes: 0,
        lock_on_app_switch: false,
    });
    assert_eq!(s.settings().idle_minutes, 1);
    s.set_settings(LockSettings {
        idle_minutes: 1_000_000,
        lock_on_app_switch: false,
    });
    assert_eq!(s.settings().idle_minutes, 60);
    s.set_settings(LockSettings {
        idle_minutes: 7,
        lock_on_app_switch: false,
    });
    assert_eq!(s.settings().idle_minutes, 5);
}

#[test]
fn changing_idle_minutes_moves_the_deadline() {
    let mut s = default_unlocked();
    s.set_settings(LockSettings {
        idle_minutes: 1,
        lock_on_app_switch: false,
    });
    assert_eq!(s.lock_deadline(), Some(T0 + MIN));
}

// ---------- Auto-lock ----------

#[test]
fn activity_pushes_the_auto_lock_deadline() {
    let mut s = default_unlocked();
    s.activity(T0 + 4 * MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 9 * MIN));
    assert_eq!(s.tick(T0 + 5 * MIN), vec![]);
    assert_eq!(s.tick(T0 + 9 * MIN), vec![Effect::Lock]);
}

#[test]
fn activity_after_the_idle_deadline_does_not_revive_the_session() {
    let mut s = default_unlocked();
    // Auto-lock was due at T0+5min, but the next tick has not run yet.
    s.activity(T0 + 5 * MIN + 500);
    assert_eq!(s.lock_deadline(), Some(T0 + 5 * MIN));
    assert_eq!(s.tick(T0 + 5 * MIN + SEC), vec![Effect::Lock]);
}

#[test]
fn activity_just_before_the_idle_deadline_still_counts() {
    let mut s = default_unlocked();
    s.activity(T0 + 5 * MIN - 1);
    assert_eq!(s.tick(T0 + 5 * MIN), vec![]);
    assert!(s.is_unlocked());
}

#[test]
fn auto_lock_respects_each_idle_choice() {
    for minutes in IDLE_MINUTE_CHOICES {
        let mut s = unlocked_session(LockSettings {
            idle_minutes: minutes,
            lock_on_app_switch: false,
        });
        let idle = Millis::from(minutes) * MIN;
        assert_eq!(s.tick(T0 + idle - 1), vec![], "{minutes} min");
        assert_eq!(s.tick(T0 + idle), vec![Effect::Lock], "{minutes} min");
        assert!(!s.is_unlocked());
    }
}

#[test]
fn tick_while_locked_does_nothing() {
    let mut s = Session::new(LockSettings::default());
    assert_eq!(s.tick(T0 + 100 * MIN), vec![]);
}

#[test]
fn clock_going_backwards_does_not_panic_or_lock() {
    let mut s = default_unlocked();
    assert_eq!(s.tick(0), vec![]);
    assert!(s.is_unlocked());
}

#[test]
fn idle_lock_also_hides_reveal_and_clears_clipboard() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "words", RevealMode::Held, T0)
        .unwrap();
    s.copied(7, T0 + 4 * MIN + 50 * SEC);
    assert_eq!(
        s.tick(T0 + 5 * MIN),
        vec![
            Effect::HideReveal,
            Effect::ClearClipboardIfUnchanged(7),
            Effect::Lock
        ]
    );
}

// ---------- OS events ----------

#[test]
fn sleep_screen_lock_user_switch_and_quit_always_lock() {
    for event in [
        OsEvent::Sleep,
        OsEvent::ScreenLocked,
        OsEvent::UserSwitched,
        OsEvent::Quit,
    ] {
        let mut s = default_unlocked();
        assert_eq!(s.os_event(event, T0 + SEC), vec![Effect::Lock], "{event:?}");
        assert!(!s.is_unlocked(), "{event:?}");
    }
}

#[test]
fn focus_gained_does_nothing() {
    let mut s = unlocked_session(LockSettings {
        idle_minutes: 5,
        lock_on_app_switch: true,
    });
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(s.os_event(OsEvent::FocusGained, T0 + SEC), vec![]);
    assert!(s.is_unlocked());
    assert!(s.current_reveal().is_some());
}

#[test]
fn quit_clears_pending_clipboard() {
    let mut s = default_unlocked();
    s.copied(9, T0);
    assert_eq!(
        s.os_event(OsEvent::Quit, T0 + SEC),
        vec![Effect::ClearClipboardIfUnchanged(9), Effect::Lock]
    );
}

#[test]
fn quit_while_locked_still_clears_a_pending_clipboard() {
    let mut s = Session::new(LockSettings::default());
    s.copied(9, T0);
    assert_eq!(
        s.os_event(OsEvent::Quit, T0 + SEC),
        vec![Effect::ClearClipboardIfUnchanged(9)]
    );
}

// ---------- Wrong credential delays ----------

fn fail_n(s: &mut Session, n: u32, at: Millis) {
    for _ in 0..n {
        s.failed_attempt(at);
    }
}

#[test]
fn first_three_attempts_have_no_delay() {
    let mut s = Session::new(LockSettings::default());
    assert_eq!(s.attempt_allowed(T0), Ok(()));
    s.failed_attempt(T0);
    assert_eq!(s.attempt_allowed(T0), Ok(()));
    s.failed_attempt(T0);
    assert_eq!(s.attempt_allowed(T0), Ok(()));
    assert_eq!(s.failed_attempts(), 2);
}

#[test]
fn delay_schedule_after_third_failure_is_1_2_5_10_then_30_seconds() {
    let expected = [1, 2, 5, 10, 30, 30, 30];
    for (i, secs) in expected.iter().enumerate() {
        let mut s = Session::new(LockSettings::default());
        fail_n(&mut s, 3 + i as u32, T0);
        assert_eq!(
            s.attempt_allowed(T0),
            Err(secs * SEC),
            "after {} failures",
            3 + i
        );
        assert_eq!(s.attempt_allowed(T0 + secs * SEC - 1), Err(1));
        assert_eq!(s.attempt_allowed(T0 + secs * SEC), Ok(()));
    }
}

#[test]
fn delay_counts_from_the_latest_failure() {
    let mut s = Session::new(LockSettings::default());
    fail_n(&mut s, 3, T0);
    s.failed_attempt(T0 + 10 * SEC);
    assert_eq!(s.attempt_allowed(T0 + 10 * SEC + 500), Err(1_500));
}

#[test]
fn many_failures_stay_capped_at_30_seconds_and_never_overflow() {
    let mut s = Session::new(LockSettings::default());
    fail_n(&mut s, 1_000, T0);
    assert_eq!(s.failed_attempts(), 1_000);
    assert_eq!(s.attempt_allowed(T0), Err(30 * SEC));
    let mut s = Session::new(LockSettings::default());
    fail_n(&mut s, 3, Millis::MAX - 1);
    assert_eq!(s.attempt_allowed(Millis::MAX - 1), Err(1));
}

#[test]
fn success_resets_failures_and_delay() {
    let mut s = Session::new(LockSettings::default());
    fail_n(&mut s, 5, T0);
    s.unlocked(T0);
    assert_eq!(s.failed_attempts(), 0);
    assert!(!s.show_recovery_hint());
    assert_eq!(s.attempt_allowed(T0), Ok(()));
}

#[test]
fn recovery_hint_shows_after_three_consecutive_failures() {
    let mut s = Session::new(LockSettings::default());
    fail_n(&mut s, 2, T0);
    assert!(!s.show_recovery_hint());
    s.failed_attempt(T0);
    assert!(s.show_recovery_hint());
}

#[test]
fn failures_survive_lock() {
    let mut s = Session::new(LockSettings::default());
    fail_n(&mut s, 4, T0);
    s.lock();
    assert_eq!(s.failed_attempts(), 4);
}

// ---------- Reveal ----------

#[test]
fn reveal_refused_while_locked() {
    let mut s = Session::new(LockSettings::default());
    assert_eq!(
        s.start_reveal("note-1", "password", RevealMode::Timed, T0),
        Err(RevealRefused::Locked)
    );
    assert_eq!(s.current_reveal(), None);
}

#[test]
fn reveal_refused_while_sharing_guard_up() {
    let mut s = default_unlocked();
    s.set_sharing_guard(true);
    assert!(s.sharing_guard_up());
    assert_eq!(
        s.start_reveal("note-1", "password", RevealMode::Timed, T0),
        Err(RevealRefused::SharingGuardUp)
    );
    s.set_sharing_guard(false);
    assert!(!s.sharing_guard_up());
    assert_eq!(
        s.start_reveal("note-1", "password", RevealMode::Timed, T0),
        Ok(())
    );
}

#[test]
fn timed_reveal_records_note_field_and_expiry() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(
        s.current_reveal(),
        Some(&Reveal {
            note_id: "note-1".into(),
            field: "password".into(),
            mode: RevealMode::Timed,
            expires_at: Some(T0 + TIMED_REVEAL_MS),
        })
    );
}

#[test]
fn timed_reveal_expires_after_30_seconds() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(s.tick(T0 + 30 * SEC - 1), vec![]);
    assert_eq!(s.tick(T0 + 30 * SEC), vec![Effect::HideReveal]);
    assert_eq!(s.current_reveal(), None);
    assert_eq!(s.tick(T0 + 31 * SEC), vec![]);
}

#[test]
fn accessible_reveal_expires_after_20_seconds() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "words", RevealMode::Accessible, T0)
        .unwrap();
    assert_eq!(
        s.current_reveal().unwrap().expires_at,
        Some(T0 + ACCESSIBLE_REVEAL_MS)
    );
    assert_eq!(s.tick(T0 + 20 * SEC - 1), vec![]);
    assert_eq!(s.tick(T0 + 20 * SEC), vec![Effect::HideReveal]);
}

#[test]
fn held_reveal_lasts_until_end_reveal() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "words", RevealMode::Held, T0)
        .unwrap();
    assert_eq!(s.current_reveal().unwrap().expires_at, None);
    s.activity(T0 + 4 * MIN);
    assert_eq!(s.tick(T0 + 4 * MIN + 30 * SEC), vec![]);
    assert!(s.current_reveal().is_some());
    assert_eq!(s.end_reveal(), vec![Effect::HideReveal]);
    assert_eq!(s.current_reveal(), None);
}

#[test]
fn opening_another_note_hides_reveal() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(s.note_opened("note-2"), vec![Effect::HideReveal]);
    assert_eq!(s.current_reveal(), None);
    assert_eq!(s.note_opened("note-3"), vec![]);
}

#[test]
fn reopening_the_same_note_keeps_reveal() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(s.note_opened("note-1"), vec![]);
    assert!(s.current_reveal().is_some());
}

#[test]
fn focus_lost_hides_reveal_without_locking() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(
        s.os_event(OsEvent::FocusLost, T0 + SEC),
        vec![Effect::HideReveal]
    );
    assert!(s.is_unlocked());
    assert_eq!(s.current_reveal(), None);
}

#[test]
fn focus_lost_with_lock_on_app_switch_hides_then_locks() {
    let mut s = unlocked_session(LockSettings {
        idle_minutes: 5,
        lock_on_app_switch: true,
    });
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(
        s.os_event(OsEvent::FocusLost, T0 + SEC),
        vec![Effect::HideReveal, Effect::Lock]
    );
    assert!(!s.is_unlocked());
}

#[test]
fn sharing_guard_going_up_hides_reveal() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Held, T0)
        .unwrap();
    assert_eq!(s.set_sharing_guard(true), vec![Effect::HideReveal]);
    assert_eq!(s.current_reveal(), None);
    assert_eq!(s.set_sharing_guard(true), vec![]);
    assert_eq!(s.set_sharing_guard(false), vec![]);
}

#[test]
fn reveal_refused_once_the_idle_deadline_has_passed() {
    let mut s = default_unlocked();
    assert_eq!(
        s.start_reveal("note-1", "password", RevealMode::Timed, T0 + 5 * MIN),
        Err(RevealRefused::Locked)
    );
    assert_eq!(s.current_reveal(), None);
    // The due Auto-lock still happens on the next tick.
    assert_eq!(s.tick(T0 + 5 * MIN + SEC), vec![Effect::Lock]);
}

#[test]
fn replacing_a_reveal_leaves_only_the_new_one_to_hide() {
    let mut s = default_unlocked();
    // The replaced one is Held (never expires), so a leftover would stay shown forever.
    s.start_reveal("note-1", "password", RevealMode::Held, T0)
        .unwrap();
    s.start_reveal("note-2", "seed-phrase", RevealMode::Timed, T0 + SEC)
        .unwrap();
    let r = s.current_reveal().unwrap();
    assert_eq!(
        (r.note_id.as_str(), r.field.as_str()),
        ("note-2", "seed-phrase")
    );
    assert_eq!(r.expires_at, Some(T0 + SEC + TIMED_REVEAL_MS));
    // One shown Hidden Field: a single hide clears it, nothing older lingers.
    assert_eq!(s.end_reveal(), vec![Effect::HideReveal]);
    assert_eq!(s.end_reveal(), vec![]);
}

#[test]
fn reveal_refused_after_lock() {
    let mut s = default_unlocked();
    s.lock();
    assert_eq!(
        s.start_reveal("note-1", "password", RevealMode::Timed, T0),
        Err(RevealRefused::Locked)
    );
}

// ---------- Clipboard ----------

#[test]
fn clipboard_clear_is_due_30_seconds_after_copy() {
    let mut s = default_unlocked();
    assert_eq!(s.clipboard_clear_at(), None);
    s.copied(100, T0);
    assert_eq!(s.clipboard_clear_at(), Some(T0 + CLIPBOARD_CLEAR_MS));
    assert_eq!(s.tick(T0 + 30 * SEC - 1), vec![]);
    assert_eq!(
        s.tick(T0 + 30 * SEC),
        vec![Effect::ClearClipboardIfUnchanged(100)]
    );
    assert_eq!(s.clipboard_clear_at(), None);
    assert_eq!(s.tick(T0 + 31 * SEC), vec![]);
}

#[test]
fn a_newer_copy_replaces_the_pending_clear() {
    let mut s = default_unlocked();
    s.copied(100, T0);
    s.copied(101, T0 + 10 * SEC);
    assert_eq!(s.clipboard_clear_at(), Some(T0 + 40 * SEC));
    assert_eq!(s.tick(T0 + 30 * SEC), vec![]);
    assert_eq!(
        s.tick(T0 + 40 * SEC),
        vec![Effect::ClearClipboardIfUnchanged(101)]
    );
}

#[test]
fn reveal_expiry_and_clipboard_clear_can_fire_on_the_same_tick() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    s.copied(5, T0);
    assert_eq!(
        s.tick(T0 + 30 * SEC),
        vec![Effect::HideReveal, Effect::ClearClipboardIfUnchanged(5)]
    );
}

// ---------- Backup reminder ----------

#[test]
fn backup_reminder_not_due_when_everything_is_backed_up() {
    assert!(!backup_reminder_due(None, T0));
}

#[test]
fn backup_reminder_due_at_seven_days() {
    assert!(!backup_reminder_due(Some(T0), T0 + BACKUP_REMINDER_MS - 1));
    assert!(backup_reminder_due(Some(T0), T0 + BACKUP_REMINDER_MS));
    assert!(backup_reminder_due(Some(T0), T0 + 30 * BACKUP_REMINDER_MS));
}

#[test]
fn backup_reminder_with_change_in_the_future_is_not_due() {
    assert!(!backup_reminder_due(Some(T0 + MIN), T0));
}
