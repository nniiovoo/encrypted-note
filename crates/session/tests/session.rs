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

/// The lock rule, with every other setting at its default.
fn lock_rule(idle_minutes: u32, lock_on_app_switch: bool) -> LockSettings {
    LockSettings {
        idle_minutes,
        lock_on_app_switch,
        ..LockSettings::default()
    }
}

/// Unlocked at [`T0`] with "count activity anywhere" on.
fn counting_anywhere(idle_minutes: u32) -> Session {
    unlocked_session(LockSettings {
        idle_minutes,
        count_activity_anywhere: true,
        ..LockSettings::default()
    })
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
            lock_on_app_switch: false,
            clipboard_clear_seconds: 30,
            count_activity_anywhere: false,
        }
    );
    let new = LockSettings {
        idle_minutes: 15,
        lock_on_app_switch: true,
        clipboard_clear_seconds: 60,
        count_activity_anywhere: true,
    };
    s.set_settings(new);
    assert_eq!(s.settings(), new);
}

#[test]
fn idle_minutes_outside_the_choices_snaps_to_nearest_choice() {
    let mut s = Session::new(lock_rule(0, false));
    assert_eq!(s.settings().idle_minutes, 1);
    s.set_settings(lock_rule(1_000_000, false));
    assert_eq!(s.settings().idle_minutes, 60);
    s.set_settings(lock_rule(7, false));
    assert_eq!(s.settings().idle_minutes, 5);
}

#[test]
fn changing_idle_minutes_moves_the_deadline() {
    let mut s = default_unlocked();
    s.set_settings(lock_rule(1, false));
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
        let mut s = unlocked_session(lock_rule(minutes, false));
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

// ---------- Activity anywhere ----------

#[test]
fn activity_elsewhere_changes_nothing_while_the_setting_is_off() {
    let mut s = default_unlocked();
    s.activity_elsewhere(T0 + 4 * MIN, T0 + 4 * MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 5 * MIN));
    assert_eq!(s.tick(T0 + 5 * MIN - 1), vec![]);
    assert_eq!(s.tick(T0 + 5 * MIN), vec![Effect::Lock]);
}

#[test]
fn with_the_setting_on_and_nothing_elsewhere_the_idle_time_is_as_before() {
    let mut s = counting_anywhere(5);
    assert_eq!(s.lock_deadline(), Some(T0 + 5 * MIN));
    s.activity(T0 + 4 * MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 9 * MIN));
    assert_eq!(s.tick(T0 + 9 * MIN - 1), vec![]);
    assert_eq!(s.tick(T0 + 9 * MIN), vec![Effect::Lock]);
}

#[test]
fn activity_elsewhere_keeps_the_app_unlocked_while_it_continues() {
    let mut s = counting_anywhere(5);
    // The owner works in another app: input every minute, nothing inside the window.
    for minute in 1..=30 {
        let now = T0 + minute * MIN;
        s.activity_elsewhere(now, now);
        assert_eq!(s.tick(now), vec![], "minute {minute}");
        assert_eq!(s.lock_deadline(), Some(now + 5 * MIN), "minute {minute}");
    }
    // It stops: the idle time runs from the last input seen anywhere.
    assert_eq!(s.tick(T0 + 35 * MIN - 1), vec![]);
    assert_eq!(s.tick(T0 + 35 * MIN), vec![Effect::Lock]);
    assert!(!s.is_unlocked());
}

#[test]
fn the_app_locks_60_minutes_after_the_last_activity_inside_the_window_whatever_happens_elsewhere() {
    let mut s = counting_anywhere(5);
    let limit = T0 + ACTIVITY_ANYWHERE_CAP_MS;
    assert_eq!(limit, T0 + 60 * MIN);
    for minute in 1..60 {
        let now = T0 + minute * MIN;
        s.activity_elsewhere(now, now);
        assert_eq!(s.tick(now), vec![], "minute {minute}");
        // The deadline is the real one: the limit, once it is nearer than the idle time.
        assert_eq!(
            s.lock_deadline(),
            Some((now + 5 * MIN).min(limit)),
            "minute {minute}"
        );
    }
    s.activity_elsewhere(limit - 1, limit - 1);
    assert_eq!(s.lock_deadline(), Some(limit));
    assert_eq!(s.tick(limit - 1), vec![]);
    assert_eq!(s.tick(limit), vec![Effect::Lock]);
    assert!(!s.is_unlocked());
}

#[test]
fn only_activity_inside_the_window_starts_the_60_minutes_again() {
    let mut s = counting_anywhere(5);
    for minute in 1..=30 {
        let now = T0 + minute * MIN;
        s.activity_elsewhere(now, now);
    }
    // The owner comes back to the app once, half an hour in, then works elsewhere again.
    s.activity(T0 + 30 * MIN);
    for minute in 31..90 {
        let now = T0 + minute * MIN;
        s.activity_elsewhere(now, now);
        assert_eq!(s.tick(now), vec![], "minute {minute}");
    }
    let limit = T0 + 30 * MIN + ACTIVITY_ANYWHERE_CAP_MS;
    assert_eq!(s.lock_deadline(), Some(limit));
    assert_eq!(s.tick(limit - 1), vec![]);
    assert_eq!(s.tick(limit), vec![Effect::Lock]);
}

#[test]
fn no_activity_report_moves_the_60_minute_limit_once_it_is_reached() {
    let mut s = counting_anywhere(5);
    let limit = T0 + ACTIVITY_ANYWHERE_CAP_MS;
    for minute in 1..60 {
        let now = T0 + minute * MIN;
        s.activity_elsewhere(now, now);
    }
    // The limit is reached before the tick that carries it out: reports of either kind,
    // from the screens or from the OS, arrive too late to move it.
    s.activity_elsewhere(limit, limit);
    s.activity(limit);
    s.activity(limit + 500);
    s.activity_elsewhere(limit + 500, limit + 500);
    assert_eq!(s.lock_deadline(), Some(limit));
    assert_eq!(s.tick(limit + SEC), vec![Effect::Lock]);
    assert!(!s.is_unlocked());
}

#[test]
fn activity_elsewhere_after_the_idle_deadline_does_not_revive_the_session() {
    let mut s = counting_anywhere(5);
    s.activity_elsewhere(T0 + 5 * MIN, T0 + 5 * MIN);
    s.activity_elsewhere(T0 + 5 * MIN + 500, T0 + 5 * MIN + 500);
    assert_eq!(s.lock_deadline(), Some(T0 + 5 * MIN));
    assert_eq!(s.tick(T0 + 5 * MIN + SEC), vec![Effect::Lock]);
}

#[test]
fn a_report_dated_in_the_future_counts_as_now_and_an_older_one_moves_nothing_back() {
    let mut s = counting_anywhere(5);
    s.activity_elsewhere(T0 + 50 * MIN, T0 + MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 6 * MIN));
    s.activity_elsewhere(T0 + 30 * SEC, T0 + 2 * MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 6 * MIN));
    // Input from before the last activity inside the window adds nothing to it.
    s.activity(T0 + 3 * MIN);
    s.activity_elsewhere(T0 + 2 * MIN, T0 + 3 * MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 8 * MIN));
}

#[test]
fn an_idle_time_of_60_minutes_is_not_stretched_by_activity_elsewhere() {
    let mut s = counting_anywhere(60);
    s.activity_elsewhere(T0 + 59 * MIN, T0 + 59 * MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 60 * MIN));
    assert_eq!(s.tick(T0 + 60 * MIN), vec![Effect::Lock]);
}

#[test]
fn turning_the_setting_off_goes_back_to_activity_inside_the_window_alone() {
    let mut s = counting_anywhere(5);
    s.activity_elsewhere(T0 + 4 * MIN, T0 + 4 * MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 9 * MIN));
    s.set_settings(LockSettings::default());
    assert_eq!(s.lock_deadline(), Some(T0 + 5 * MIN));
    // Turning it on again does not bring the earlier input back.
    s.set_settings(LockSettings {
        count_activity_anywhere: true,
        ..LockSettings::default()
    });
    assert_eq!(s.lock_deadline(), Some(T0 + 5 * MIN));
    assert_eq!(s.tick(T0 + 5 * MIN), vec![Effect::Lock]);
}

#[test]
fn sleep_screen_lock_user_switch_and_quit_lock_at_once_while_activity_anywhere_is_counted() {
    for event in [
        OsEvent::Sleep,
        OsEvent::ScreenLocked,
        OsEvent::UserSwitched,
        OsEvent::Quit,
    ] {
        let mut s = counting_anywhere(5);
        s.activity_elsewhere(T0 + SEC, T0 + SEC);
        assert_eq!(
            s.os_event(event, T0 + 2 * SEC),
            vec![Effect::Lock],
            "{event:?}"
        );
        assert!(!s.is_unlocked(), "{event:?}");
        // Input elsewhere while Locked unlocks nothing and starts no timer.
        s.activity_elsewhere(T0 + 3 * SEC, T0 + 3 * SEC);
        assert!(!s.is_unlocked(), "{event:?}");
        assert_eq!(s.lock_deadline(), None, "{event:?}");
    }
    // "Lock when I switch apps" and the owner's own Lock are not held back either.
    let mut s = unlocked_session(LockSettings {
        lock_on_app_switch: true,
        count_activity_anywhere: true,
        ..LockSettings::default()
    });
    s.activity_elsewhere(T0 + SEC, T0 + SEC);
    assert_eq!(
        s.os_event(OsEvent::FocusLost, T0 + 2 * SEC),
        vec![Effect::Lock]
    );
    let mut s = counting_anywhere(5);
    s.activity_elsewhere(T0 + SEC, T0 + SEC);
    assert_eq!(s.lock(), vec![Effect::Lock]);
    // After the next unlock the idle time starts from that unlock.
    s.unlocked(T0 + 10 * MIN);
    assert_eq!(s.lock_deadline(), Some(T0 + 15 * MIN));
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
    let mut s = unlocked_session(lock_rule(5, true));
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
            extensions: 0,
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
    assert_eq!(s.tick(T0 + HELD_REVEAL_MAX_MS - 1), vec![]);
    assert!(s.current_reveal().is_some());
    assert_eq!(s.end_reveal(), vec![Effect::HideReveal]);
    assert_eq!(s.current_reveal(), None);
}

#[test]
fn held_reveal_is_hidden_after_two_minutes_if_the_release_never_arrives() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "words", RevealMode::Held, T0)
        .unwrap();
    // Key auto-repeat keeps reporting activity; that must not keep the value on screen.
    s.activity(T0 + MIN);
    assert_eq!(s.tick(T0 + HELD_REVEAL_MAX_MS), vec![Effect::HideReveal]);
    assert_eq!(s.current_reveal(), None);
    assert!(s.is_unlocked());

    // The backstop belongs to that reveal only: a later timed reveal keeps its own 30 s.
    s.start_reveal("note-1", "password", RevealMode::Timed, T0 + 3 * MIN)
        .unwrap();
    assert_eq!(s.tick(T0 + 3 * MIN + TIMED_REVEAL_MS - 1), vec![]);
    assert_eq!(
        s.tick(T0 + 3 * MIN + TIMED_REVEAL_MS),
        vec![Effect::HideReveal]
    );
}

#[test]
fn a_ninth_different_field_within_a_minute_is_refused() {
    let mut s = default_unlocked();
    for i in 0..MAX_FIELDS_PER_WINDOW {
        let at = T0 + i as Millis * SEC;
        s.start_reveal(&format!("note-{i}"), "password", RevealMode::Timed, at)
            .unwrap();
    }
    let now = T0 + 10 * SEC;
    assert_eq!(
        s.start_reveal("note-x", "password", RevealMode::Timed, now),
        Err(RevealRefused::TooMany(REVEAL_WINDOW_MS - 10 * SEC))
    );
    // The refused field is not shown, and the one shown before stays as it was.
    assert_eq!(s.current_reveal().unwrap().note_id, "note-7");
    // Showing a field again that was already shown costs nothing.
    s.start_reveal("note-0", "password", RevealMode::Timed, now)
        .unwrap();
    // Locking and unlocking does not start a fresh allowance.
    s.lock();
    s.unlocked(now);
    assert!(matches!(
        s.start_reveal("note-x", "password", RevealMode::Timed, now),
        Err(RevealRefused::TooMany(_))
    ));
    // The repeated field renewed its slot; the second field is now the first to expire.
    assert_eq!(
        s.start_reveal(
            "note-x",
            "password",
            RevealMode::Timed,
            T0 + REVEAL_WINDOW_MS
        ),
        Err(RevealRefused::TooMany(SEC))
    );
    s.start_reveal(
        "note-x",
        "password",
        RevealMode::Timed,
        T0 + REVEAL_WINDOW_MS + SEC,
    )
    .unwrap();
}

#[test]
fn disclosure_admission_checks_access_without_starting_a_reveal() {
    let mut s = Session::new(LockSettings::default());
    assert_eq!(
        s.admit_hidden_field("note-1", "key", T0),
        Err(RevealRefused::Locked)
    );
    s.unlocked(T0);
    s.set_sharing_guard(true);
    assert_eq!(
        s.admit_hidden_field("note-1", "key", T0),
        Err(RevealRefused::SharingGuardUp)
    );
    s.set_sharing_guard(false);
    s.admit_hidden_field("note-1", "key", T0).unwrap();
    assert_eq!(s.current_reveal(), None);
    s.start_reveal("note-1", "key", RevealMode::Timed, T0)
        .unwrap();
    let shown = s.current_reveal().cloned();
    // Field names matter as well as Note ids, and copies leave the shown field alone.
    for i in 1..MAX_FIELDS_PER_WINDOW {
        s.admit_hidden_field("note-1", &format!("field-{i}"), T0)
            .unwrap();
    }
    assert_eq!(s.current_reveal().cloned(), shown);
    assert_eq!(
        s.admit_hidden_field("note-2", "key", T0),
        Err(RevealRefused::TooMany(REVEAL_WINDOW_MS))
    );
    assert_eq!(
        s.admit_hidden_field("note-1", "key", T0 + 5 * MIN),
        Err(RevealRefused::Locked)
    );
}

#[test]
fn repeating_fields_renews_their_slots_without_allowing_a_rolling_window_bypass() {
    let mut s = default_unlocked();
    for i in 0..MAX_FIELDS_PER_WINDOW {
        s.admit_hidden_field(&format!("note-{i}"), "key", T0)
            .unwrap();
    }
    // Reusing every field costs no extra slots, but keeps each in the rolling window.
    for i in 0..MAX_FIELDS_PER_WINDOW {
        s.start_reveal(
            &format!("note-{i}"),
            "key",
            RevealMode::Timed,
            T0 + REVEAL_WINDOW_MS - SEC,
        )
        .unwrap();
    }
    assert_eq!(
        s.admit_hidden_field("note-x", "key", T0 + REVEAL_WINDOW_MS),
        Err(RevealRefused::TooMany(REVEAL_WINDOW_MS - SEC))
    );
    assert_eq!(
        s.admit_hidden_field("note-x", "key", T0 + 2 * REVEAL_WINDOW_MS - SEC - 1),
        Err(RevealRefused::TooMany(1))
    );
    s.admit_hidden_field("note-x", "key", T0 + 2 * REVEAL_WINDOW_MS - SEC)
        .unwrap();
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
    let mut s = unlocked_session(lock_rule(5, true));
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(
        s.os_event(OsEvent::FocusLost, T0 + SEC),
        vec![Effect::HideReveal, Effect::Lock]
    );
    assert!(!s.is_unlocked());
}

#[test]
fn locking_on_app_switch_leaves_a_copy_on_its_30_second_timer() {
    let mut s = unlocked_session(lock_rule(5, true));
    s.copied(9, T0);
    // Switching to the other app to paste locks, but must not wipe the copy at once.
    assert_eq!(s.os_event(OsEvent::FocusLost, T0 + SEC), vec![Effect::Lock]);
    assert!(!s.is_unlocked());
    assert_eq!(s.clipboard_clear_at(), Some(T0 + CLIPBOARD_CLEAR_MS));
    assert_eq!(s.tick(T0 + CLIPBOARD_CLEAR_MS - 1), vec![]);
    assert_eq!(
        s.tick(T0 + CLIPBOARD_CLEAR_MS),
        vec![Effect::ClearClipboardIfUnchanged(9)]
    );
    // Sleep, screen lock, user switch and quit still clear it at once.
    for event in [OsEvent::Sleep, OsEvent::ScreenLocked, OsEvent::Quit] {
        let mut s = Session::new(LockSettings::default());
        s.copied(9, T0);
        assert_eq!(
            s.os_event(event, T0 + SEC),
            vec![Effect::ClearClipboardIfUnchanged(9)],
            "{event:?}"
        );
    }
}

#[test]
fn restored_failures_keep_their_delay_after_a_restart() {
    let mut s = Session::new(LockSettings::default());
    s.restore_failures(5, T0);
    assert_eq!(s.failed_attempts(), 5);
    assert!(s.show_recovery_hint());
    // Five failures earn the 5 s delay, counted from the restart.
    assert_eq!(s.attempt_allowed(T0), Err(5 * SEC));
    assert_eq!(s.attempt_allowed(T0 + 5 * SEC), Ok(()));
    // The first three attempts never wait.
    let mut s = Session::new(LockSettings::default());
    s.restore_failures(2, T0);
    assert_eq!(s.attempt_allowed(T0), Ok(()));
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

// ---------- More time for a shown value ----------

#[test]
fn a_timed_reveal_gets_a_new_30_seconds_as_often_as_asked() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    let mut now = T0;
    for round in 1..=10u32 {
        now += 20 * SEC;
        s.activity(now);
        assert_eq!(s.extend_reveal(now), Ok(()), "round {round}");
        let reveal = s.current_reveal().unwrap();
        // Counted from the moment of asking, not added to what was left.
        assert_eq!(reveal.expires_at, Some(now + TIMED_REVEAL_MS));
        assert_eq!(reveal.extensions, round);
        assert!(reveal.can_extend());
        assert_eq!(reveal.extensions_left(), None);
    }
    assert_eq!(s.tick(now + TIMED_REVEAL_MS - 1), vec![]);
    assert_eq!(s.tick(now + TIMED_REVEAL_MS), vec![Effect::HideReveal]);
    assert_eq!(s.current_reveal(), None);
}

#[test]
fn a_wallet_kind_reveal_gets_a_new_20_seconds_three_times_then_has_to_be_shown_again() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "words", RevealMode::Accessible, T0)
        .unwrap();
    assert!(s.current_reveal().unwrap().can_extend());
    assert_eq!(s.current_reveal().unwrap().extensions_left(), Some(3));
    let mut now = T0;
    for used in 1..=MAX_ACCESSIBLE_EXTENSIONS {
        now += 15 * SEC;
        assert_eq!(s.extend_reveal(now), Ok(()), "extension {used}");
        let reveal = s.current_reveal().unwrap();
        assert_eq!(reveal.expires_at, Some(now + ACCESSIBLE_REVEAL_MS));
        assert_eq!(
            reveal.extensions_left(),
            Some(MAX_ACCESSIBLE_EXTENSIONS - used)
        );
        assert_eq!(reveal.can_extend(), used < MAX_ACCESSIBLE_EXTENSIONS);
    }
    // A fourth time is refused and changes nothing: the value hides 20 s after the third.
    assert_eq!(s.extend_reveal(now + SEC), Err(ExtendRefused::LimitReached));
    assert_eq!(
        s.current_reveal().unwrap().expires_at,
        Some(now + ACCESSIBLE_REVEAL_MS)
    );
    assert_eq!(s.tick(now + ACCESSIBLE_REVEAL_MS - 1), vec![]);
    assert_eq!(s.tick(now + ACCESSIBLE_REVEAL_MS), vec![Effect::HideReveal]);
    assert_eq!(
        s.extend_reveal(now + ACCESSIBLE_REVEAL_MS),
        Err(ExtendRefused::NothingShown)
    );

    // Showing it again (the app asks for the Master Password first) starts a new count.
    let again = now + ACCESSIBLE_REVEAL_MS + SEC;
    s.start_reveal("note-1", "words", RevealMode::Accessible, again)
        .unwrap();
    assert_eq!(s.current_reveal().unwrap().extensions_left(), Some(3));
    assert_eq!(s.extend_reveal(again + SEC), Ok(()));
}

#[test]
fn a_held_reveal_cannot_be_given_more_time() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "words", RevealMode::Held, T0)
        .unwrap();
    assert!(!s.current_reveal().unwrap().can_extend());
    assert_eq!(s.current_reveal().unwrap().extensions_left(), None);
    assert_eq!(s.extend_reveal(T0 + SEC), Err(ExtendRefused::Held));
    assert_eq!(
        s.extend_reveal(T0 + HELD_REVEAL_MAX_MS - 1),
        Err(ExtendRefused::Held)
    );
    let reveal = s.current_reveal().unwrap();
    assert_eq!((reveal.expires_at, reveal.extensions), (None, 0));
    // Its two-minute backstop is where it was.
    assert_eq!(s.tick(T0 + HELD_REVEAL_MAX_MS - 1), vec![]);
    assert_eq!(s.tick(T0 + HELD_REVEAL_MAX_MS), vec![Effect::HideReveal]);
}

#[test]
fn a_reveal_that_has_ended_is_never_brought_back() {
    // Nothing was shown at all.
    let mut s = default_unlocked();
    assert_eq!(s.extend_reveal(T0), Err(ExtendRefused::NothingShown));

    // Its time ran out and a tick hid it.
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    assert_eq!(s.tick(T0 + TIMED_REVEAL_MS), vec![Effect::HideReveal]);
    assert_eq!(
        s.extend_reveal(T0 + TIMED_REVEAL_MS),
        Err(ExtendRefused::NothingShown)
    );
    assert_eq!(s.current_reveal(), None);

    // Its time is up, but the tick that hides it has not run yet.
    let shown = T0 + MIN;
    s.start_reveal("note-1", "password", RevealMode::Timed, shown)
        .unwrap();
    assert_eq!(s.extend_reveal(shown + TIMED_REVEAL_MS - 1), Ok(()));
    let ends = shown + 2 * TIMED_REVEAL_MS - 1;
    assert_eq!(s.extend_reveal(ends), Err(ExtendRefused::NothingShown));
    assert_eq!(s.current_reveal().unwrap().expires_at, Some(ends));
    assert_eq!(s.tick(ends), vec![Effect::HideReveal]);

    // Every other way a reveal ends.
    type End = fn(&mut Session);
    let ways: [(&str, End); 5] = [
        ("Hide", |s| drop(s.end_reveal())),
        ("another Note opened", |s| drop(s.note_opened("note-2"))),
        ("focus lost", |s| {
            drop(s.os_event(OsEvent::FocusLost, T0 + SEC))
        }),
        ("Sharing Guard up", |s| drop(s.set_sharing_guard(true))),
        ("lock", |s| drop(s.lock())),
    ];
    for (name, end) in ways {
        for mode in [RevealMode::Timed, RevealMode::Accessible] {
            let mut s = default_unlocked();
            s.start_reveal("note-1", "password", mode, T0).unwrap();
            end(&mut s);
            assert_eq!(
                s.extend_reveal(T0 + 2 * SEC),
                Err(ExtendRefused::NothingShown),
                "{name}, {mode:?}"
            );
            assert_eq!(s.current_reveal(), None, "{name}, {mode:?}");
        }
    }
}

#[test]
fn more_time_is_refused_once_the_auto_lock_is_due() {
    let mut s = unlocked_session(lock_rule(1, false));
    // Shown 10 s before the idle time is up: the lock comes first.
    s.start_reveal("note-1", "password", RevealMode::Timed, T0 + 50 * SEC)
        .unwrap();
    assert_eq!(s.extend_reveal(T0 + MIN), Err(ExtendRefused::NothingShown));
    assert_eq!(s.tick(T0 + MIN), vec![Effect::HideReveal, Effect::Lock]);
    assert_eq!(
        s.extend_reveal(T0 + MIN + SEC),
        Err(ExtendRefused::NothingShown)
    );
}

#[test]
fn more_time_is_not_activity_and_an_extended_reveal_ends_like_any_other() {
    let mut s = default_unlocked();
    s.start_reveal("note-1", "password", RevealMode::Timed, T0)
        .unwrap();
    s.copied(7, T0);
    assert_eq!(s.extend_reveal(T0 + 10 * SEC), Ok(()));
    // Neither the idle time nor the clipboard timer moved.
    assert_eq!(s.lock_deadline(), Some(T0 + 5 * MIN));
    assert_eq!(s.clipboard_clear_at(), Some(T0 + CLIPBOARD_CLEAR_MS));
    assert_eq!(s.note_opened("note-2"), vec![Effect::HideReveal]);

    for mode in [RevealMode::Timed, RevealMode::Accessible] {
        let mut s = default_unlocked();
        s.start_reveal("note-1", "password", mode, T0).unwrap();
        assert_eq!(s.extend_reveal(T0 + SEC), Ok(()));
        assert_eq!(
            s.os_event(OsEvent::FocusLost, T0 + 2 * SEC),
            vec![Effect::HideReveal],
            "{mode:?}"
        );
        s.start_reveal("note-1", "password", mode, T0 + 3 * SEC)
            .unwrap();
        assert_eq!(s.extend_reveal(T0 + 4 * SEC), Ok(()));
        assert_eq!(s.set_sharing_guard(true), vec![Effect::HideReveal]);
        s.set_sharing_guard(false);
        s.start_reveal("note-1", "password", mode, T0 + 5 * SEC)
            .unwrap();
        assert_eq!(s.extend_reveal(T0 + 6 * SEC), Ok(()));
        assert_eq!(s.lock(), vec![Effect::HideReveal, Effect::Lock], "{mode:?}");
    }
}

#[test]
fn more_time_does_not_use_up_or_refill_the_eight_fields_a_minute() {
    let mut s = default_unlocked();
    for i in 0..MAX_FIELDS_PER_WINDOW {
        let at = T0 + i as Millis * SEC;
        s.start_reveal(&format!("note-{i}"), "password", RevealMode::Timed, at)
            .unwrap();
    }
    let now = T0 + 10 * SEC;
    for _ in 0..5 {
        assert_eq!(s.extend_reveal(now), Ok(()));
    }
    assert_eq!(s.current_reveal().unwrap().note_id, "note-7");
    // A ninth different field is refused exactly as it would have been.
    assert_eq!(
        s.start_reveal("note-x", "password", RevealMode::Timed, now),
        Err(RevealRefused::TooMany(REVEAL_WINDOW_MS - 10 * SEC))
    );
    // The refusal left the extended reveal alone.
    assert_eq!(
        s.current_reveal().unwrap().expires_at,
        Some(now + TIMED_REVEAL_MS)
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
fn failed_clipboard_clear_remains_scheduled_after_lock() {
    let mut s = default_unlocked();
    s.copied(100, T0);
    assert_eq!(
        s.lock(),
        vec![Effect::ClearClipboardIfUnchanged(100), Effect::Lock]
    );
    s.retry_clipboard_clear(100, T0 + SEC);
    assert_eq!(s.clipboard_clear_at(), Some(T0 + SEC + 250));
    assert_eq!(s.tick(T0 + SEC + 249), vec![]);
    assert_eq!(
        s.tick(T0 + SEC + 250),
        vec![Effect::ClearClipboardIfUnchanged(100)]
    );
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

/// Unlocked at [`T0`] with the clipboard cleared after `seconds`.
fn clearing_after(seconds: u32, lock_on_app_switch: bool) -> Session {
    unlocked_session(LockSettings {
        lock_on_app_switch,
        clipboard_clear_seconds: seconds,
        ..LockSettings::default()
    })
}

#[test]
fn a_copy_is_cleared_after_60_seconds_when_the_owner_chose_that() {
    let mut s = clearing_after(60, false);
    assert_eq!(s.settings().clipboard_clear_seconds, 60);
    s.copied(100, T0);
    assert_eq!(s.clipboard_clear_at(), Some(T0 + 60 * SEC));
    assert_eq!(s.tick(T0 + 60 * SEC - 1), vec![]);
    assert_eq!(
        s.tick(T0 + 60 * SEC),
        vec![Effect::ClearClipboardIfUnchanged(100)]
    );
    assert_eq!(s.clipboard_clear_at(), None);
}

#[test]
fn a_wallet_kind_copy_is_always_cleared_after_30_seconds() {
    for seconds in CLIPBOARD_CLEAR_SECOND_CHOICES {
        let mut s = clearing_after(seconds, false);
        s.copied_wallet_value(5, T0);
        assert_eq!(
            s.clipboard_clear_at(),
            Some(T0 + CLIPBOARD_CLEAR_MS),
            "{seconds} s chosen"
        );
        assert_eq!(s.tick(T0 + 30 * SEC - 1), vec![]);
        assert_eq!(
            s.tick(T0 + 30 * SEC),
            vec![Effect::ClearClipboardIfUnchanged(5)]
        );
        // The setting itself is as chosen: the next ordinary copy uses it.
        s.copied(6, T0 + MIN);
        assert_eq!(
            s.clipboard_clear_at(),
            Some(T0 + MIN + Millis::from(seconds) * SEC)
        );
    }
}

#[test]
fn a_clipboard_time_that_is_not_one_of_the_two_choices_becomes_30_seconds() {
    for chosen in [0, 1, 29, 31, 45, 59, 61, 120, 3_600, u32::MAX] {
        let settings = LockSettings {
            clipboard_clear_seconds: chosen,
            ..LockSettings::default()
        };
        let mut s = Session::new(settings);
        assert_eq!(s.settings().clipboard_clear_seconds, 30, "{chosen}");
        s.set_settings(LockSettings {
            clipboard_clear_seconds: 60,
            ..LockSettings::default()
        });
        s.set_settings(settings);
        assert_eq!(s.settings().clipboard_clear_seconds, 30, "{chosen}");
        s.unlocked(T0);
        s.copied(1, T0);
        assert_eq!(s.clipboard_clear_at(), Some(T0 + 30 * SEC), "{chosen}");
    }
    for chosen in CLIPBOARD_CLEAR_SECOND_CHOICES {
        let s = clearing_after(chosen, false);
        assert_eq!(s.settings().clipboard_clear_seconds, chosen);
    }
    assert_eq!(CLIPBOARD_CLEAR_SECOND_CHOICES, [30, 60]);
    assert_eq!(DEFAULT_CLIPBOARD_CLEAR_SECONDS, 30);
}

#[test]
fn changing_the_clipboard_time_leaves_a_pending_clear_where_it_was() {
    let mut s = clearing_after(60, false);
    s.copied(100, T0);
    s.set_settings(LockSettings::default());
    assert_eq!(s.clipboard_clear_at(), Some(T0 + 60 * SEC));
    s.copied(101, T0 + 10 * SEC);
    assert_eq!(s.clipboard_clear_at(), Some(T0 + 40 * SEC));
}

#[test]
fn locking_on_app_switch_leaves_a_60_second_copy_on_its_timer_too() {
    let mut s = clearing_after(60, true);
    s.copied(9, T0);
    assert_eq!(s.os_event(OsEvent::FocusLost, T0 + SEC), vec![Effect::Lock]);
    assert!(!s.is_unlocked());
    assert_eq!(s.clipboard_clear_at(), Some(T0 + 60 * SEC));
    assert_eq!(s.tick(T0 + 60 * SEC - 1), vec![]);
    assert_eq!(
        s.tick(T0 + 60 * SEC),
        vec![Effect::ClearClipboardIfUnchanged(9)]
    );

    // A Wallet Kind copy keeps its own 30 s through the same lock.
    let mut s = clearing_after(60, true);
    s.copied_wallet_value(9, T0);
    assert_eq!(s.os_event(OsEvent::FocusLost, T0 + SEC), vec![Effect::Lock]);
    assert_eq!(s.tick(T0 + 30 * SEC - 1), vec![]);
    assert_eq!(
        s.tick(T0 + 30 * SEC),
        vec![Effect::ClearClipboardIfUnchanged(9)]
    );

    // Every other lock still clears it at once, whichever time was chosen.
    for seconds in CLIPBOARD_CLEAR_SECOND_CHOICES {
        for event in [
            OsEvent::Sleep,
            OsEvent::ScreenLocked,
            OsEvent::UserSwitched,
            OsEvent::Quit,
        ] {
            let mut s = clearing_after(seconds, true);
            s.copied(9, T0);
            assert_eq!(
                s.os_event(event, T0 + SEC),
                vec![Effect::ClearClipboardIfUnchanged(9), Effect::Lock],
                "{event:?}, {seconds} s"
            );
        }
        let mut s = clearing_after(seconds, true);
        s.copied(9, T0);
        assert_eq!(
            s.lock(),
            vec![Effect::ClearClipboardIfUnchanged(9), Effect::Lock]
        );
        // And so does the idle time running out.
        let mut s = clearing_after(seconds, true);
        s.copied(9, T0 + 5 * MIN - SEC);
        assert_eq!(
            s.tick(T0 + 5 * MIN),
            vec![Effect::ClearClipboardIfUnchanged(9), Effect::Lock]
        );
    }
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
