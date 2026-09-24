mod common;

use std::collections::BTreeMap;

use common::*;
use notes::{
    Document, Filter, Kind, NewNote, NoteEdit, NotesError, SCHEMA_VERSION, TRASH_RETENTION_SECS,
    field,
};
use serde_json::json;

const T0: i64 = 1_700_000_000;

fn doc() -> Document {
    Document::new(T0)
}

fn titles(list: &[notes::NoteSummary]) -> Vec<&str> {
    list.iter().map(|s| s.title.as_str()).collect()
}

// ---------- creating ----------

#[test]
fn new_document_is_empty() {
    let d = doc();
    assert_eq!(d.count(), 0);
    assert_eq!(d.changed_at(), T0);
    assert!(d.list(Filter::All, "").is_empty());
}

#[test]
fn note_ids_are_32_lowercase_hex_and_unique() {
    let mut d = doc();
    let a = d.create(text("A", "fake body a"), None, T0).unwrap();
    let b = d.create(text("B", "fake body b"), None, T0).unwrap();
    for id in [&a, &b] {
        assert_eq!(id.len(), 32);
        assert!(
            id.chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
        );
    }
    assert_ne!(a, b);
}

#[test]
fn create_rejects_blank_title() {
    let mut d = doc();
    assert_eq!(
        d.create(text("   ", "fake"), None, T0),
        Err(NotesError::EmptyTitle)
    );
    assert_eq!(d.count(), 0);
}

#[test]
fn create_trims_the_title() {
    let mut d = doc();
    let id = d.create(text("  Shopping  ", "fake"), None, T0).unwrap();
    assert_eq!(d.get(&id).unwrap().title, "Shopping");
}

#[test]
fn create_rejects_fields_that_do_not_belong_to_the_kind() {
    let mut d = doc();
    let mut note = text("T", "fake");
    note.visible
        .insert(field::WEBSITE.into(), "example.test".into());
    assert_eq!(
        d.create(note, None, T0),
        Err(NotesError::UnknownField(field::WEBSITE.into()))
    );

    let mut note = login("L", "example.test", "user", "fake-pw");
    note.hidden
        .insert(field::BODY.into(), zeroize::Zeroizing::new("x".into()));
    assert_eq!(
        d.create(note, None, T0),
        Err(NotesError::UnknownField(field::BODY.into()))
    );
    // A visible field name used as hidden is also rejected.
    let mut note = login("L", "example.test", "user", "fake-pw");
    note.hidden
        .insert(field::USERNAME.into(), zeroize::Zeroizing::new("x".into()));
    assert_eq!(
        d.create(note, None, T0),
        Err(NotesError::UnknownField(field::USERNAME.into()))
    );
    assert_eq!(d.count(), 0);
}

#[test]
fn wallet_kinds_need_a_wallet_cipher_to_create() {
    let mut d = doc();
    assert_eq!(
        d.create(seed("S", "Main", SEED_12), None, T0),
        Err(NotesError::NeedsWalletKey)
    );
    assert_eq!(
        d.create(
            private_key("P", "evm", "0xFAKEADDRESS", "fake-key"),
            None,
            T0
        ),
        Err(NotesError::NeedsWalletKey)
    );
    assert_eq!(d.count(), 0);
}

#[test]
fn seed_phrase_must_have_12_or_24_words() {
    let mut d = doc();
    let eleven = ["abandon"; 11].join(" ");
    assert_eq!(
        d.create(seed("S", "W", &eleven), Some(&CIPHER), T0),
        Err(NotesError::BadWordCount(11))
    );
    let no_words = NewNote {
        kind: Kind::SeedPhrase,
        title: "S".into(),
        visible: BTreeMap::new(),
        hidden: BTreeMap::new(),
    };
    assert_eq!(
        d.create(no_words, Some(&CIPHER), T0),
        Err(NotesError::BadWordCount(0))
    );
    assert!(
        d.create(seed("S12", "W", SEED_12), Some(&CIPHER), T0)
            .is_ok()
    );
    assert!(
        d.create(seed("S24", "W", SEED_24), Some(&CIPHER), T0)
            .is_ok()
    );
}

#[test]
fn seed_phrase_that_fails_the_checksum_can_still_be_saved() {
    let mut d = doc();
    let bad = ["abandon"; 12].join(" ");
    assert!(d.create(seed("S", "W", &bad), Some(&CIPHER), T0).is_ok());
}

#[test]
fn word_count_comes_from_the_words_not_the_input() {
    let mut d = doc();
    let mut note = seed("S", "Main", SEED_24);
    note.visible.insert(field::WORD_COUNT.into(), "12".into());
    let id = d.create(note, Some(&CIPHER), T0).unwrap();
    assert_eq!(
        d.get(&id)
            .unwrap()
            .visible
            .get(field::WORD_COUNT)
            .map(String::as_str),
        Some("24")
    );
}

#[test]
fn seed_words_are_stored_normalized() {
    let mut d = doc();
    let messy = format!("  {}  ", SEED_12.to_uppercase().replace(' ', "   "));
    let id = d
        .create(seed("S", "Main", &messy), Some(&CIPHER), T0)
        .unwrap();
    assert_eq!(
        d.reveal_wallet(&id, field::WORDS, &CIPHER)
            .unwrap()
            .as_str(),
        SEED_12
    );
}

// ---------- viewing and revealing ----------

#[test]
fn get_shows_visible_fields_and_which_hidden_fields_have_values() {
    let mut d = doc();
    let id = d
        .create(seed("Cold wallet", "Main", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    let view = d.get(&id).unwrap();
    assert_eq!(view.kind, Kind::SeedPhrase);
    assert_eq!(view.title, "Cold wallet");
    assert_eq!(
        view.visible,
        map(&[(field::WALLET_NAME, "Main"), (field::WORD_COUNT, "12")])
    );
    assert_eq!(
        view.hidden,
        vec![
            (field::WORDS.to_string(), true),
            (field::PASSPHRASE.to_string(), false)
        ]
    );
    assert!(!view.favorite);
    assert_eq!(
        (view.created_at, view.updated_at, view.deleted_at),
        (T0, T0, None)
    );
    assert!(d.get("0000000000000000000000000000dead").is_none());
}

#[test]
fn reveal_errors() {
    let mut d = doc();
    let t = d
        .create(
            NewNote {
                kind: Kind::Text,
                title: "Empty".into(),
                visible: BTreeMap::new(),
                hidden: BTreeMap::new(),
            },
            None,
            T0,
        )
        .unwrap();
    assert_eq!(d.reveal(&t, field::BODY), Err(NotesError::EmptyField));
    assert_eq!(
        d.reveal(&t, field::PASSWORD),
        Err(NotesError::UnknownField(field::PASSWORD.into()))
    );
    assert_eq!(d.reveal("nope", field::BODY), Err(NotesError::NotFound));
    let s = d
        .create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    assert_eq!(d.reveal(&s, field::WORDS), Err(NotesError::NeedsWalletKey));
}

#[test]
fn reveal_wallet_opens_with_the_cipher() {
    let mut d = doc();
    let mut note = seed("S", "W", SEED_24);
    note.hidden.insert(
        field::PASSPHRASE.into(),
        zeroize::Zeroizing::new("fake extra word".into()),
    );
    let s = d.create(note, Some(&CIPHER), T0).unwrap();
    let p = d
        .create(
            private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    assert_eq!(
        d.reveal_wallet(&s, field::WORDS, &CIPHER).unwrap().as_str(),
        SEED_24
    );
    assert_eq!(
        d.reveal_wallet(&s, field::PASSPHRASE, &CIPHER)
            .unwrap()
            .as_str(),
        "fake extra word"
    );
    assert_eq!(
        d.reveal_wallet(&p, field::KEY, &CIPHER).unwrap().as_str(),
        "fake-private-key-1"
    );
}

#[test]
fn reveal_wallet_with_the_wrong_key_fails() {
    let mut d = doc();
    let s = d
        .create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    assert_eq!(
        d.reveal_wallet(&s, field::WORDS, &FakeCipher { key: 1 }),
        Err(NotesError::WalletCipher)
    );
}

#[test]
fn reveal_wallet_errors() {
    let mut d = doc();
    let s = d
        .create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    assert_eq!(
        d.reveal_wallet(&s, field::PASSPHRASE, &CIPHER),
        Err(NotesError::EmptyField)
    );
    assert_eq!(
        d.reveal_wallet(&s, field::BODY, &CIPHER),
        Err(NotesError::UnknownField(field::BODY.into()))
    );
    assert_eq!(
        d.reveal_wallet("nope", field::WORDS, &CIPHER),
        Err(NotesError::NotFound)
    );
    let t = d.create(text("T", "fake body"), None, T0).unwrap();
    assert!(d.reveal_wallet(&t, field::BODY, &CIPHER).is_err());
}

#[test]
fn wallet_hidden_fields_never_appear_in_plain_in_the_body() {
    let mut d = doc();
    let s = d
        .create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    d.create(
        private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
        Some(&CIPHER),
        T0,
    )
    .unwrap();
    let json = String::from_utf8(d.to_json().to_vec()).unwrap();
    assert!(!json.contains("abandon"));
    assert!(!json.contains("fake-private-key-1"));
    // Round trip still opens them.
    let back = Document::from_json(json.as_bytes()).unwrap();
    assert_eq!(
        back.reveal_wallet(&s, field::WORDS, &CIPHER)
            .unwrap()
            .as_str(),
        SEED_12
    );
}

// ---------- editing ----------

#[test]
fn edit_keeps_hidden_fields_that_are_not_supplied() {
    let mut d = doc();
    let id = d
        .create(
            login(
                "Mail",
                "mail.example.test",
                "someone",
                "correct-horse-test-password-1",
            ),
            None,
            T0,
        )
        .unwrap();
    let edit = NoteEdit {
        title: Some("Mail (work)".into()),
        visible: map(&[(field::USERNAME, "someone-else")]),
        ..Default::default()
    };
    d.update(&id, edit, None, T0 + 5).unwrap();
    let view = d.get(&id).unwrap();
    assert_eq!(view.title, "Mail (work)");
    assert_eq!(
        view.visible,
        map(&[
            (field::WEBSITE, "mail.example.test"),
            (field::USERNAME, "someone-else")
        ])
    );
    assert_eq!(view.updated_at, T0 + 5);
    assert_eq!(view.created_at, T0);
    assert_eq!(
        d.reveal(&id, field::PASSWORD).unwrap().as_str(),
        "correct-horse-test-password-1"
    );
}

#[test]
fn edit_replaces_a_hidden_field_when_a_new_value_is_supplied() {
    let mut d = doc();
    let id = d
        .create(api_key("CI", "ci.example.test", "fake-token-old"), None, T0)
        .unwrap();
    d.update(
        &id,
        NoteEdit {
            hidden: hidden(&[(field::KEY, "fake-token-new")]),
            ..Default::default()
        },
        None,
        T0,
    )
    .unwrap();
    assert_eq!(
        d.reveal(&id, field::KEY).unwrap().as_str(),
        "fake-token-new"
    );
}

#[test]
fn edit_with_an_empty_hidden_value_clears_it() {
    let mut d = doc();
    let id = d.create(text("T", "fake body"), None, T0).unwrap();
    d.update(
        &id,
        NoteEdit {
            hidden: hidden(&[(field::BODY, "")]),
            ..Default::default()
        },
        None,
        T0,
    )
    .unwrap();
    assert_eq!(d.reveal(&id, field::BODY), Err(NotesError::EmptyField));
    assert_eq!(
        d.get(&id).unwrap().hidden,
        vec![(field::BODY.to_string(), false)]
    );
}

#[test]
fn edit_with_an_empty_visible_value_clears_it() {
    let mut d = doc();
    let id = d
        .create(login("L", "site.example.test", "me", "fake-pw"), None, T0)
        .unwrap();
    d.update(
        &id,
        NoteEdit {
            visible: map(&[(field::USERNAME, "")]),
            ..Default::default()
        },
        None,
        T0,
    )
    .unwrap();
    assert_eq!(
        d.get(&id).unwrap().visible,
        map(&[(field::WEBSITE, "site.example.test")])
    );
}

#[test]
fn editing_only_visible_parts_of_a_wallet_kind_needs_no_cipher() {
    let mut d = doc();
    let id = d
        .create(
            private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    let edit = NoteEdit {
        title: Some("Hot wallet".into()),
        visible: map(&[(field::ADDRESS, "0xFAKE2")]),
        ..Default::default()
    };
    d.update(&id, edit, None, T0).unwrap();
    assert_eq!(
        d.get(&id)
            .unwrap()
            .visible
            .get(field::ADDRESS)
            .map(String::as_str),
        Some("0xFAKE2")
    );
    assert_eq!(
        d.reveal_wallet(&id, field::KEY, &CIPHER).unwrap().as_str(),
        "fake-private-key-1"
    );
}

#[test]
fn editing_a_wallet_hidden_field_needs_a_cipher() {
    let mut d = doc();
    let id = d
        .create(
            private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    let edit = || NoteEdit {
        hidden: hidden(&[(field::KEY, "fake-private-key-2")]),
        ..Default::default()
    };
    assert_eq!(
        d.update(&id, edit(), None, T0),
        Err(NotesError::NeedsWalletKey)
    );
    assert_eq!(
        d.reveal_wallet(&id, field::KEY, &CIPHER).unwrap().as_str(),
        "fake-private-key-1"
    );
    d.update(&id, edit(), Some(&CIPHER), T0).unwrap();
    assert_eq!(
        d.reveal_wallet(&id, field::KEY, &CIPHER).unwrap().as_str(),
        "fake-private-key-2"
    );
}

#[test]
fn editing_seed_words_rechecks_the_count_and_updates_word_count() {
    let mut d = doc();
    let id = d
        .create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    let bad = NoteEdit {
        title: Some("Renamed".into()),
        hidden: hidden(&[(field::WORDS, "abandon about")]),
        ..Default::default()
    };
    assert_eq!(
        d.update(&id, bad, Some(&CIPHER), T0),
        Err(NotesError::BadWordCount(2))
    );
    // Nothing changed, not even the title.
    assert_eq!(d.get(&id).unwrap().title, "S");
    d.update(
        &id,
        NoteEdit {
            hidden: hidden(&[(field::WORDS, SEED_24)]),
            ..Default::default()
        },
        Some(&CIPHER),
        T0,
    )
    .unwrap();
    assert_eq!(
        d.get(&id)
            .unwrap()
            .visible
            .get(field::WORD_COUNT)
            .map(String::as_str),
        Some("24")
    );
    assert_eq!(
        d.reveal_wallet(&id, field::WORDS, &CIPHER)
            .unwrap()
            .as_str(),
        SEED_24
    );
}

#[test]
fn edit_errors_leave_the_note_unchanged() {
    let mut d = doc();
    let id = d
        .create(login("L", "site.example.test", "me", "fake-pw"), None, T0)
        .unwrap();
    assert_eq!(
        d.update("nope", NoteEdit::default(), None, T0),
        Err(NotesError::NotFound)
    );
    let blank = NoteEdit {
        title: Some("  ".into()),
        ..Default::default()
    };
    assert_eq!(d.update(&id, blank, None, T0), Err(NotesError::EmptyTitle));
    let wrong = NoteEdit {
        title: Some("New".into()),
        visible: map(&[(field::SERVICE, "x")]),
        ..Default::default()
    };
    assert_eq!(
        d.update(&id, wrong, None, T0),
        Err(NotesError::UnknownField(field::SERVICE.into()))
    );
    let wrong = NoteEdit {
        hidden: hidden(&[(field::KEY, "x")]),
        ..Default::default()
    };
    assert_eq!(
        d.update(&id, wrong, None, T0),
        Err(NotesError::UnknownField(field::KEY.into()))
    );
    assert_eq!(d.get(&id).unwrap().title, "L");
}

// ---------- change tracking ----------

#[test]
fn every_mutation_bumps_changed_at_and_the_change_counter() {
    let mut d = doc();
    let mut last = d.change_counter();
    let mut t = T0;
    let mut step = |d: &Document, t: i64| {
        assert_eq!(d.changed_at(), t);
        assert!(d.change_counter() > last, "counter did not grow");
        last = d.change_counter();
    };
    t += 1;
    let id = d.create(text("T", "fake"), None, t).unwrap();
    step(&d, t);
    t += 1;
    d.update(
        &id,
        NoteEdit {
            title: Some("T2".into()),
            ..Default::default()
        },
        None,
        t,
    )
    .unwrap();
    step(&d, t);
    t += 1;
    d.set_favorite(&id, true, t).unwrap();
    step(&d, t);
    t += 1;
    d.trash(&id, t).unwrap();
    step(&d, t);
    t += 1;
    d.restore(&id, t).unwrap();
    step(&d, t);
    t += 1;
    d.trash(&id, t).unwrap();
    step(&d, t);
    t += 1;
    d.delete_forever(&id, t).unwrap();
    step(&d, t);
    let s = d.create(seed("S", "W", SEED_12), Some(&CIPHER), t).unwrap();
    t += 1;
    d.reencrypt_wallet_fields(&CIPHER, &FakeCipher { key: 9 }, t)
        .unwrap();
    step(&d, t);
    t += 1;
    d.trash(&s, t).unwrap();
    t += 1;
    assert_eq!(d.empty_trash(t), 1);
    step(&d, t);
    let x = d.create(text("X", "fake"), None, t).unwrap();
    d.trash(&x, t).unwrap();
    t += TRASH_RETENTION_SECS;
    assert_eq!(d.purge_expired(t), 1);
    step(&d, t);
}

#[test]
fn failed_or_empty_operations_do_not_count_as_changes() {
    let mut d = doc();
    let id = d.create(text("T", "fake"), None, T0).unwrap();
    let (at, n) = (d.changed_at(), d.change_counter());
    assert!(
        d.update(
            &id,
            NoteEdit {
                title: Some(" ".into()),
                ..Default::default()
            },
            None,
            T0 + 9
        )
        .is_err()
    );
    assert!(d.trash("nope", T0 + 9).is_err());
    assert!(d.delete_forever(&id, T0 + 9).is_err());
    assert_eq!(d.purge_expired(T0 + 9), 0);
    assert_eq!(d.empty_trash(T0 + 9), 0);
    assert_eq!((d.changed_at(), d.change_counter()), (at, n));
}

#[test]
fn operations_that_change_nothing_do_not_count_as_changes() {
    let mut d = doc();
    let id = d.create(text("T", "fake"), None, T0).unwrap();
    let unchanged = |d: &Document, at: i64, n: u64| {
        assert_eq!((d.changed_at(), d.change_counter()), (at, n));
    };
    let (at, n) = (d.changed_at(), d.change_counter());
    d.restore(&id, T0 + 1).unwrap();
    d.set_favorite(&id, false, T0 + 1).unwrap();
    unchanged(&d, at, n);

    d.set_favorite(&id, true, T0 + 2).unwrap();
    let (at, n) = (d.changed_at(), d.change_counter());
    d.set_favorite(&id, true, T0 + 3).unwrap();
    unchanged(&d, at, n);

    d.trash(&id, T0 + 4).unwrap();
    let (at, n) = (d.changed_at(), d.change_counter());
    d.trash(&id, T0 + 5).unwrap();
    unchanged(&d, at, n);
    assert_eq!(d.list(Filter::Trash, "")[0].deleted_at, Some(T0 + 4));
}

#[test]
fn older_is_decided_by_changed_at_then_change_counter() {
    let mut a = Document::new(T0);
    let mut b = Document::new(T0);
    assert!(!a.is_older_than(&b) && !b.is_older_than(&a));
    b.create(text("T", "fake"), None, T0).unwrap();
    assert!(a.is_older_than(&b));
    assert!(!b.is_older_than(&a));
    a.create(text("T", "fake"), None, T0 + 1).unwrap();
    assert!(b.is_older_than(&a));
}

// ---------- Trash ----------

#[test]
fn trashed_notes_leave_the_main_list_and_can_be_restored() {
    let mut d = doc();
    let id = d.create(text("T", "fake"), None, T0).unwrap();
    d.trash(&id, T0 + 10).unwrap();
    assert_eq!(d.count(), 0);
    assert!(d.list(Filter::All, "").is_empty());
    let trash = d.list(Filter::Trash, "");
    assert_eq!(titles(&trash), vec!["T"]);
    assert_eq!(trash[0].deleted_at, Some(T0 + 10));
    assert_eq!(d.get(&id).unwrap().deleted_at, Some(T0 + 10));
    d.restore(&id, T0 + 20).unwrap();
    assert_eq!(d.count(), 1);
    assert!(d.list(Filter::Trash, "").is_empty());
    assert_eq!(d.get(&id).unwrap().deleted_at, None);
    assert_eq!(d.restore("nope", T0), Err(NotesError::NotFound));
}

#[test]
fn delete_forever_only_works_on_notes_in_trash() {
    let mut d = doc();
    let id = d.create(text("T", "fake"), None, T0).unwrap();
    assert_eq!(d.delete_forever(&id, T0), Err(NotesError::NotFound));
    assert!(d.get(&id).is_some());
    d.trash(&id, T0).unwrap();
    d.delete_forever(&id, T0).unwrap();
    assert!(d.get(&id).is_none());
    assert!(d.list(Filter::Trash, "").is_empty());
}

#[test]
fn empty_trash_removes_only_trashed_notes() {
    let mut d = doc();
    let keep = d.create(text("Keep", "fake"), None, T0).unwrap();
    for t in ["A", "B"] {
        let id = d.create(text(t, "fake"), None, T0).unwrap();
        d.trash(&id, T0).unwrap();
    }
    assert_eq!(d.empty_trash(T0 + 1), 2);
    assert_eq!(d.count(), 1);
    assert!(d.get(&keep).is_some());
    assert!(d.list(Filter::Trash, "").is_empty());
}

#[test]
fn trash_is_purged_30_days_after_deletion() {
    let mut d = doc();
    let old = d.create(text("Old", "fake"), None, T0).unwrap();
    let newer = d.create(text("Newer", "fake"), None, T0).unwrap();
    let live = d.create(text("Live", "fake"), None, T0).unwrap();
    d.trash(&old, T0).unwrap();
    d.trash(&newer, T0 + 100).unwrap();
    assert_eq!(d.purge_expired(T0 + TRASH_RETENTION_SECS - 1), 0);
    assert_eq!(d.purge_expired(T0 + TRASH_RETENTION_SECS), 1);
    assert!(d.get(&old).is_none());
    assert!(d.get(&newer).is_some());
    assert_eq!(d.purge_expired(T0 + TRASH_RETENTION_SECS + 100), 1);
    assert!(d.get(&newer).is_none());
    assert!(d.get(&live).is_some());
    assert_eq!(d.purge_expired(i64::MAX), 0);
}

// ---------- listing, Favorites and search ----------

#[test]
fn favorites_come_first_then_most_recently_updated() {
    let mut d = doc();
    let a = d.create(text("A", "fake"), None, T0 + 1).unwrap();
    let _b = d.create(text("B", "fake"), None, T0 + 2).unwrap();
    let _c = d.create(text("C", "fake"), None, T0 + 3).unwrap();
    assert_eq!(titles(&d.list(Filter::All, "")), vec!["C", "B", "A"]);
    d.set_favorite(&a, true, T0 + 4).unwrap();
    let list = d.list(Filter::All, "");
    assert_eq!(titles(&list), vec!["A", "C", "B"]);
    assert!(list[0].favorite);
    assert_eq!(titles(&d.list(Filter::Favorites, "")), vec!["A"]);
    d.set_favorite(&a, false, T0 + 5).unwrap();
    assert!(d.list(Filter::Favorites, "").is_empty());
    assert_eq!(d.set_favorite("nope", true, T0), Err(NotesError::NotFound));
}

#[test]
fn trashed_favorites_are_not_listed_as_favorites() {
    let mut d = doc();
    let a = d.create(text("A", "fake"), None, T0).unwrap();
    d.set_favorite(&a, true, T0).unwrap();
    d.trash(&a, T0).unwrap();
    assert!(d.list(Filter::Favorites, "").is_empty());
}

#[test]
fn filter_by_kind() {
    let mut d = doc();
    d.create(text("T", "fake"), None, T0).unwrap();
    d.create(login("L", "site.example.test", "me", "fake-pw"), None, T0)
        .unwrap();
    d.create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    assert_eq!(titles(&d.list(Filter::Kind(Kind::Login), "")), vec!["L"]);
    assert_eq!(
        titles(&d.list(Filter::Kind(Kind::SeedPhrase), "")),
        vec!["S"]
    );
    assert!(d.list(Filter::Kind(Kind::ApiKey), "").is_empty());
}

#[test]
fn trash_is_ordered_by_deletion_newest_first() {
    let mut d = doc();
    let a = d.create(text("A", "fake"), None, T0 + 30).unwrap();
    let b = d.create(text("B", "fake"), None, T0).unwrap();
    d.set_favorite(&b, true, T0).unwrap();
    d.trash(&a, T0 + 40).unwrap();
    d.trash(&b, T0 + 50).unwrap();
    assert_eq!(titles(&d.list(Filter::Trash, "")), vec!["B", "A"]);
}

#[test]
fn search_is_case_insensitive_over_title_and_visible_fields() {
    let mut d = doc();
    d.create(
        login("Mail", "mail.example.test", "someone", "fake-pw"),
        None,
        T0,
    )
    .unwrap();
    d.create(api_key("CI", "Builds.Example.test", "fake-token"), None, T0)
        .unwrap();
    d.create(
        private_key("P", "evm", "0xFAKEADDR", "fake-key"),
        Some(&CIPHER),
        T0,
    )
    .unwrap();
    d.create(seed("S", "Cold Storage", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    assert_eq!(titles(&d.list(Filter::All, "MAIL")), vec!["Mail"]);
    assert_eq!(titles(&d.list(Filter::All, "SomeOne")), vec!["Mail"]);
    assert_eq!(titles(&d.list(Filter::All, "builds")), vec!["CI"]);
    assert_eq!(titles(&d.list(Filter::All, "fakeaddr")), vec!["P"]);
    assert_eq!(titles(&d.list(Filter::All, "EVM")), vec!["P"]);
    assert_eq!(titles(&d.list(Filter::All, "cold stor")), vec!["S"]);
    assert_eq!(d.list(Filter::All, "").len(), 4);
    assert!(d.list(Filter::All, "zzz-no-match").is_empty());
}

#[test]
fn search_never_matches_hidden_fields() {
    let mut d = doc();
    d.create(
        login(
            "L",
            "site.example.test",
            "me",
            "correct-horse-test-password-1",
        ),
        None,
        T0,
    )
    .unwrap();
    d.create(text("T", "secret-body-marker"), None, T0).unwrap();
    d.create(api_key("A", "svc", "fake-token-marker"), None, T0)
        .unwrap();
    d.create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    d.create(
        private_key("P", "other", "addr", "fake-pk-marker"),
        Some(&CIPHER),
        T0,
    )
    .unwrap();
    for q in [
        "correct-horse",
        "body-marker",
        "token-marker",
        "abandon",
        "pk-marker",
    ] {
        assert!(
            d.list(Filter::All, q).is_empty(),
            "query {q:?} matched a Hidden Field"
        );
    }
}

#[test]
fn search_applies_within_the_filter() {
    let mut d = doc();
    let a = d.create(text("Alpha", "fake"), None, T0).unwrap();
    d.create(text("Alphabet", "fake"), None, T0).unwrap();
    d.trash(&a, T0).unwrap();
    assert_eq!(titles(&d.list(Filter::Trash, "alpha")), vec!["Alpha"]);
    assert_eq!(titles(&d.list(Filter::All, "alpha")), vec!["Alphabet"]);
}

#[test]
fn summary_subtitle_is_the_first_visible_detail() {
    let mut d = doc();
    d.create(login("L", "site.example.test", "me", "fake-pw"), None, T0)
        .unwrap();
    let mut no_site = login("L2", "", "only-user", "fake-pw");
    no_site.visible.remove(field::WEBSITE);
    d.create(no_site, None, T0 + 1).unwrap();
    d.create(
        private_key("P", "evm", "0xFAKEADDR", "fake-key"),
        Some(&CIPHER),
        T0 + 2,
    )
    .unwrap();
    d.create(seed("S", "Cold", SEED_12), Some(&CIPHER), T0 + 3)
        .unwrap();
    d.create(api_key("A", "svc.example.test", "fake-token"), None, T0 + 4)
        .unwrap();
    d.create(text("T", "fake"), None, T0 + 5).unwrap();
    let subs: Vec<(String, Option<String>)> = d
        .list(Filter::All, "")
        .into_iter()
        .map(|s| (s.title, s.subtitle))
        .collect();
    let expect = |t: &str, s: Option<&str>| (t.to_string(), s.map(String::from));
    assert_eq!(
        subs,
        vec![
            expect("T", None),
            expect("A", Some("svc.example.test")),
            expect("S", Some("Cold")),
            expect("P", Some("0xFAKEADDR")),
            expect("L2", Some("only-user")),
            expect("L", Some("site.example.test")),
        ]
    );
}

// ---------- serialisation ----------

#[test]
fn json_round_trip_keeps_everything() {
    let mut d = doc();
    let l = d
        .create(
            login(
                "Mail",
                "mail.example.test",
                "someone",
                "correct-horse-test-password-1",
            ),
            None,
            T0,
        )
        .unwrap();
    let s = d
        .create(seed("S", "W", SEED_12), Some(&CIPHER), T0 + 1)
        .unwrap();
    let t = d.create(text("T", "fake body"), None, T0 + 2).unwrap();
    d.set_favorite(&l, true, T0 + 3).unwrap();
    d.trash(&t, T0 + 4).unwrap();

    let back = Document::from_json(&d.to_json()).unwrap();
    assert_eq!(back.changed_at(), d.changed_at());
    assert_eq!(back.change_counter(), d.change_counter());
    assert_eq!(back.count(), d.count());
    for f in [Filter::All, Filter::Favorites, Filter::Trash] {
        assert_eq!(back.list(f, ""), d.list(f, ""));
    }
    for id in [&l, &s, &t] {
        assert_eq!(back.get(id), d.get(id));
    }
    assert_eq!(
        back.reveal(&l, field::PASSWORD).unwrap().as_str(),
        "correct-horse-test-password-1"
    );
    assert_eq!(back.reveal(&t, field::BODY).unwrap().as_str(), "fake body");
    assert_eq!(
        back.reveal_wallet(&s, field::WORDS, &CIPHER)
            .unwrap()
            .as_str(),
        SEED_12
    );
    assert!(!back.is_older_than(&d) && !d.is_older_than(&back));
}

#[test]
fn to_json_writes_into_one_exact_allocation() {
    // A growing buffer would free earlier copies of the plaintext without wiping them.
    let mut d = doc();
    d.create(text("T", &"x".repeat(10_000)), None, T0).unwrap();
    d.create(
        login(
            "L",
            "site.example.test",
            "u",
            "correct-horse-test-password-1",
        ),
        None,
        T0,
    )
    .unwrap();
    let out = d.to_json();
    assert!(out.len() > 10_000);
    assert_eq!(out.capacity(), out.len());
    assert!(Document::from_json(&out).is_ok());
}

fn body_with_note(note: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": SCHEMA_VERSION, "changed_at": T0, "change_counter": 1, "notes": [note]
    }))
    .unwrap()
}

fn raw_note(
    kind: &str,
    visible: serde_json::Value,
    hidden: serde_json::Value,
    sealed: serde_json::Value,
) -> serde_json::Value {
    json!({
        "id": "00112233445566778899aabbccddeeff", "kind": kind, "title": "T",
        "visible": visible, "hidden": hidden, "sealed": sealed,
        "favorite": false, "created_at": T0, "updated_at": T0, "deleted_at": null
    })
}

#[test]
fn a_hand_built_body_in_the_documented_shape_parses() {
    let note = raw_note(
        "login",
        json!({"website": "site.example.test"}),
        json!({"password": "fake-pw"}),
        json!({}),
    );
    let d = Document::from_json(&body_with_note(note)).unwrap();
    assert_eq!(
        d.reveal("00112233445566778899aabbccddeeff", field::PASSWORD)
            .unwrap()
            .as_str(),
        "fake-pw"
    );
}

#[test]
fn from_json_rejects_garbage() {
    for input in [&b"not json"[..], b""] {
        assert!(matches!(
            Document::from_json(input),
            Err(NotesError::Corrupt(_))
        ));
    }
}

#[test]
fn from_json_rejects_kind_and_field_mismatches() {
    let empty = || json!({});
    let bad = [
        // Visible field from another Kind.
        raw_note("text", json!({"website": "x"}), empty(), empty()),
        // Hidden field from another Kind.
        raw_note("login", empty(), json!({"body": "x"}), empty()),
        // Wallet Kind with a plain Hidden Field.
        raw_note("private_key", empty(), json!({"key": "fake"}), empty()),
        // Ordinary Kind with a sealed field.
        raw_note("text", empty(), empty(), json!({"body": "00ff"})),
        // Sealed value that isn't hex.
        raw_note("private_key", empty(), empty(), json!({"key": "zz"})),
        // Unknown Kind.
        raw_note("recipe", empty(), empty(), empty()),
    ];
    for note in bad {
        let r = Document::from_json(&body_with_note(note.clone()));
        assert!(matches!(r, Err(NotesError::Corrupt(_))), "accepted {note}");
    }
}

#[test]
fn from_json_rejects_bodies_that_editing_could_never_produce() {
    let empty = || json!({});
    let sealed_words = || json!({"words": "00ff"});
    let wc = |n: &str| json!({"wallet_name": "W", "word_count": n});
    let mut blank_title = raw_note("text", empty(), json!({"body": "x"}), empty());
    blank_title["title"] = "   ".into();
    let bad = [
        blank_title,
        // Seed Phrase with no words at all.
        raw_note("seed_phrase", wc("12"), empty(), empty()),
        // Seed Phrase with an empty sealed value.
        raw_note("seed_phrase", wc("12"), empty(), json!({"words": ""})),
        // word_count that isn't 12 or 24, or missing.
        raw_note("seed_phrase", wc("7"), empty(), sealed_words()),
        raw_note(
            "seed_phrase",
            json!({"wallet_name": "W"}),
            empty(),
            sealed_words(),
        ),
        // Empty plain Hidden Field value.
        raw_note("login", empty(), json!({"password": ""}), empty()),
        // Empty sealed value.
        raw_note("private_key", empty(), empty(), json!({"key": ""})),
        // Empty visible value.
        raw_note("login", json!({"website": ""}), empty(), empty()),
    ];
    for note in bad {
        let r = Document::from_json(&body_with_note(note.clone()));
        assert!(matches!(r, Err(NotesError::Corrupt(_))), "accepted {note}");
    }
    // The well-formed Seed Phrase shape still parses.
    let good = raw_note("seed_phrase", wc("24"), empty(), sealed_words());
    assert!(Document::from_json(&body_with_note(good)).is_ok());
}

#[test]
fn corrupt_errors_never_echo_values_from_the_input() {
    const FAKE: &str = "FAKE-PASSWORD-VALUE";
    let login = |hidden: serde_json::Value| raw_note("login", json!({}), hidden, json!({}));
    let future = |note: serde_json::Value| {
        serde_json::to_vec(&json!({
            "schema_version": SCHEMA_VERSION + 1, "changed_at": T0, "change_counter": 0, "notes": [note]
        }))
        .unwrap()
    };
    let mut bad_kind = login(json!({}));
    bad_kind["kind"] = FAKE.into();
    let mut bad_id = login(json!({}));
    bad_id["id"] = FAKE.into();
    let mut wrong_type = login(json!({"password": FAKE}));
    wrong_type["favorite"] = FAKE.into();
    let inputs: Vec<Vec<u8>> = vec![
        // A Backup from a newer schema whose shape this build doesn't know.
        future(login(FAKE.into())),
        body_with_note(login(FAKE.into())),
        body_with_note(login(json!({"password": [FAKE]}))),
        body_with_note(bad_kind),
        body_with_note(bad_id),
        body_with_note(wrong_type),
        format!(r#"{{"schema_version":"{FAKE}"}}"#).into_bytes(),
        format!(r#"{{"schema_version":1,"{FAKE}":1}}"#).into_bytes(),
        format!(r#"{{"schema_version":1,"notes":[{{"hidden":{{"password":"{FAKE}"#).into_bytes(),
    ];
    for input in inputs {
        let e = Document::from_json(&input).err().expect("must be rejected");
        assert!(matches!(e, NotesError::Corrupt(_)), "{e:?}");
        assert!(!format!("{e:?}").contains(FAKE), "{e:?}");
    }
    // A newer schema is reported as such, not as a parse failure.
    match Document::from_json(&future(login(FAKE.into()))) {
        Err(NotesError::Corrupt(m)) => assert!(m.contains("schema version"), "{m}"),
        other => panic!("{:?}", other.err()),
    }
}

#[test]
fn from_json_rejects_duplicate_ids() {
    let note = raw_note("text", json!({}), json!({}), json!({}));
    let body = serde_json::to_vec(&json!({
        "schema_version": SCHEMA_VERSION, "changed_at": T0, "change_counter": 1, "notes": [note.clone(), note]
    }))
    .unwrap();
    assert!(matches!(
        Document::from_json(&body),
        Err(NotesError::Corrupt(_))
    ));
}

// ---------- Key Rotation ----------

#[test]
fn reencrypting_moves_wallet_fields_to_the_new_key() {
    let mut d = doc();
    let mut note = seed("S", "W", SEED_12);
    note.hidden.insert(
        field::PASSPHRASE.into(),
        zeroize::Zeroizing::new("fake extra word".into()),
    );
    let s = d.create(note, Some(&CIPHER), T0).unwrap();
    let p = d
        .create(
            private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    let l = d
        .create(login("L", "site.example.test", "me", "fake-pw"), None, T0)
        .unwrap();
    let new = FakeCipher { key: 0x33 };
    d.reencrypt_wallet_fields(&CIPHER, &new, T0 + 1).unwrap();
    assert_eq!(
        d.reveal_wallet(&s, field::WORDS, &new).unwrap().as_str(),
        SEED_12
    );
    assert_eq!(
        d.reveal_wallet(&s, field::PASSPHRASE, &new)
            .unwrap()
            .as_str(),
        "fake extra word"
    );
    assert_eq!(
        d.reveal_wallet(&p, field::KEY, &new).unwrap().as_str(),
        "fake-private-key-1"
    );
    assert_eq!(
        d.reveal_wallet(&p, field::KEY, &CIPHER),
        Err(NotesError::WalletCipher)
    );
    assert_eq!(d.reveal(&l, field::PASSWORD).unwrap().as_str(), "fake-pw");
}

#[test]
fn reencrypting_with_the_wrong_old_key_changes_nothing() {
    let mut d = doc();
    let s = d
        .create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    let n = d.change_counter();
    let wrong = FakeCipher { key: 7 };
    assert_eq!(
        d.reencrypt_wallet_fields(&wrong, &FakeCipher { key: 8 }, T0 + 1),
        Err(NotesError::WalletCipher)
    );
    assert_eq!(
        d.reveal_wallet(&s, field::WORDS, &CIPHER).unwrap().as_str(),
        SEED_12
    );
    assert_eq!(d.change_counter(), n);
}
