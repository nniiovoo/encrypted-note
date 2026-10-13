mod common;

use std::collections::BTreeMap;

use common::*;
use notes::{
    BASE_SCHEMA_VERSION, Document, Filter, Kind, NewNote, NoteEdit, NotesError, SCHEMA_VERSION,
    TRASH_RETENTION_SECS, field,
};
use serde_json::json;

const T0: i64 = 1_700_000_000;

fn doc() -> Document {
    Document::new(T0)
}

fn titles(list: &[notes::NoteSummary]) -> Vec<&str> {
    list.iter().map(|s| s.title.as_str()).collect()
}

/// A new Note of any Kind with exactly these fields.
fn new_note(
    kind: Kind,
    title: &str,
    visible: &[(&str, &str)],
    hidden_values: &[(&str, &str)],
) -> NewNote {
    NewNote {
        kind,
        title: title.into(),
        visible: map(visible),
        hidden: hidden(hidden_values),
    }
}

/// An edit that sets these visible values and nothing else.
fn set_visible(pairs: &[(&str, &str)]) -> NoteEdit {
    NoteEdit {
        visible: map(pairs),
        ..Default::default()
    }
}

/// An edit that sets these Hidden Field values and nothing else.
fn set_hidden(pairs: &[(&str, &str)]) -> NoteEdit {
    NoteEdit {
        hidden: hidden(pairs),
        ..Default::default()
    }
}

/// The `schema_version` a document is written with.
fn written_schema(d: &Document) -> u64 {
    let json: serde_json::Value = serde_json::from_slice(&d.to_json()).unwrap();
    json["schema_version"].as_u64().unwrap()
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
fn visible_values_are_trimmed_and_a_blank_one_removes_the_field() {
    let mut d = doc();
    let id = d
        .create(
            login("L", "  site.example.test \n", " \t ", "fake-pw"),
            None,
            T0,
        )
        .unwrap();
    // The blank username was never stored.
    assert_eq!(
        d.get(&id).unwrap().visible,
        map(&[(field::WEBSITE, "site.example.test")])
    );
    d.update(&id, set_visible(&[(field::USERNAME, "\tme  ")]), None, T0)
        .unwrap();
    assert_eq!(
        d.get(&id).unwrap().visible,
        map(&[
            (field::WEBSITE, "site.example.test"),
            (field::USERNAME, "me")
        ])
    );
    // Whitespace inside a value is kept.
    d.update(&id, set_visible(&[(field::USERNAME, " me too ")]), None, T0)
        .unwrap();
    assert_eq!(d.get(&id).unwrap().visible[field::USERNAME], "me too");
    // A value of only whitespace removes the field, like an empty one.
    d.update(&id, set_visible(&[(field::WEBSITE, " \n ")]), None, T0)
        .unwrap();
    assert_eq!(
        d.get(&id).unwrap().visible,
        map(&[(field::USERNAME, "me too")])
    );
    // What is written never holds an empty value, so it parses again.
    assert!(Document::from_json(&d.to_json()).is_ok());
}

/// Fake PEM text: the dashes and labels of a real block around a body that is not a key.
const FAKE_PEM: &str =
    "-----BEGIN FAKE PRIVATE KEY-----\nZmFrZS1ub3QtYS1rZXk=\n-----END FAKE PRIVATE KEY-----\n";

#[test]
fn stray_whitespace_around_a_pasted_key_is_not_stored() {
    let mut d = doc();
    let a = d
        .create(
            new_note(
                Kind::ApiKey,
                "A",
                &[(field::SERVICE, "svc.example.test")],
                &[
                    (field::KEY, "  fake-api-key-0000 \t"),
                    (field::SECRET, "\tfake-api-secret-0000 "),
                ],
            ),
            None,
            T0,
        )
        .unwrap();
    assert_eq!(
        d.reveal(&a, field::KEY).unwrap().as_str(),
        "fake-api-key-0000"
    );
    assert_eq!(
        d.reveal(&a, field::SECRET).unwrap().as_str(),
        "fake-api-secret-0000"
    );
    let p = d
        .create(
            private_key("P", "evm", "0xFAKE", " fake-private-key-1  "),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    assert_eq!(
        d.reveal_wallet(&p, field::KEY, &CIPHER).unwrap().as_str(),
        "fake-private-key-1"
    );

    // The same when editing.
    d.update(
        &a,
        set_hidden(&[
            (field::KEY, "\u{a0}fake-api-key-1111  "),
            (field::SECRET, "fake-api-secret-1111   "),
        ]),
        None,
        T0,
    )
    .unwrap();
    assert_eq!(
        d.reveal(&a, field::KEY).unwrap().as_str(),
        "fake-api-key-1111"
    );
    assert_eq!(
        d.reveal(&a, field::SECRET).unwrap().as_str(),
        "fake-api-secret-1111"
    );
    d.update(
        &p,
        set_hidden(&[(field::KEY, "   fake-private-key-2")]),
        Some(&CIPHER),
        T0,
    )
    .unwrap();
    assert_eq!(
        d.reveal_wallet(&p, field::KEY, &CIPHER).unwrap().as_str(),
        "fake-private-key-2"
    );

    // Whitespace inside a key is part of it, and a key without any around it is untouched.
    d.update(
        &a,
        set_hidden(&[(field::KEY, " fake key\twith spaces ")]),
        None,
        T0,
    )
    .unwrap();
    assert_eq!(
        d.reveal(&a, field::KEY).unwrap().as_str(),
        "fake key\twith spaces"
    );
}

#[test]
fn a_line_break_only_before_or_after_a_one_line_key_is_stray_whitespace_too() {
    // Copying a key out of a file or a terminal usually brings its line ending along.
    let mut d = doc();
    let a = d
        .create(
            api_key("A", "svc.example.test", "fake-api-key-0000\n"),
            None,
            T0,
        )
        .unwrap();
    assert_eq!(
        d.reveal(&a, field::KEY).unwrap().as_str(),
        "fake-api-key-0000"
    );
    d.update(
        &a,
        set_hidden(&[(field::KEY, "\r\n fake-api-key-1111 \r\n\r\n")]),
        None,
        T0,
    )
    .unwrap();
    assert_eq!(
        d.reveal(&a, field::KEY).unwrap().as_str(),
        "fake-api-key-1111"
    );
}

#[test]
fn a_key_of_several_lines_is_stored_exactly_as_received() {
    let crlf = FAKE_PEM.replace('\n', "\r\n");
    let padded = format!("  {FAKE_PEM}\n ");
    let two_lines = " fake line one \n fake line two ";
    for value in [FAKE_PEM, crlf.as_str(), padded.as_str(), two_lines] {
        let mut d = doc();
        let a = d
            .create(
                new_note(
                    Kind::ApiKey,
                    "A",
                    &[(field::SERVICE, "svc.example.test")],
                    &[(field::KEY, value), (field::SECRET, value)],
                ),
                None,
                T0,
            )
            .unwrap();
        assert_eq!(d.reveal(&a, field::KEY).unwrap().as_str(), value);
        assert_eq!(d.reveal(&a, field::SECRET).unwrap().as_str(), value);
        let p = d
            .create(private_key("P", "other", "", value), Some(&CIPHER), T0)
            .unwrap();
        assert_eq!(
            d.reveal_wallet(&p, field::KEY, &CIPHER).unwrap().as_str(),
            value
        );
        // And when it replaces a one-line key in an edit, and after a round trip.
        let b = d
            .create(api_key("B", "svc.example.test", "fake-token"), None, T0)
            .unwrap();
        d.update(&b, set_hidden(&[(field::KEY, value)]), None, T0)
            .unwrap();
        let back = Document::from_json(&d.to_json()).unwrap();
        assert_eq!(back.reveal(&b, field::KEY).unwrap().as_str(), value);
    }
}

#[test]
fn a_pasted_key_that_is_only_whitespace_counts_as_not_supplied() {
    let mut d = doc();
    // Creating: the field is simply absent.
    let blank = d
        .create(
            new_note(
                Kind::ApiKey,
                "Blank",
                &[(field::SERVICE, "svc.example.test")],
                &[(field::KEY, " \t "), (field::SECRET, "\n")],
            ),
            None,
            T0,
        )
        .unwrap();
    assert_eq!(
        d.get(&blank).unwrap().hidden,
        vec![
            (field::KEY.to_string(), false),
            (field::SECRET.to_string(), false)
        ]
    );
    let blank_pk = d
        .create(
            private_key("Blank key", "evm", "0xFAKE", "   "),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    assert_eq!(
        d.get(&blank_pk).unwrap().hidden,
        vec![(field::KEY.to_string(), false)]
    );

    // Editing: the stored value is kept, never cleared and never replaced by the blanks.
    let a = d
        .create(
            new_note(
                Kind::ApiKey,
                "A",
                &[(field::SERVICE, "svc.example.test")],
                &[
                    (field::KEY, "fake-api-key-0000"),
                    (field::SECRET, "fake-api-secret-0000"),
                ],
            ),
            None,
            T0,
        )
        .unwrap();
    for blanks in ["   ", "\t", "\n", " \r\n \n", "\u{a0}\u{2003}"] {
        d.update(
            &a,
            set_hidden(&[(field::KEY, blanks), (field::SECRET, blanks)]),
            None,
            T0 + 1,
        )
        .unwrap();
        assert_eq!(
            d.reveal(&a, field::KEY).unwrap().as_str(),
            "fake-api-key-0000",
            "{blanks:?}"
        );
        assert_eq!(
            d.reveal(&a, field::SECRET).unwrap().as_str(),
            "fake-api-secret-0000",
            "{blanks:?}"
        );
    }
    let p = d
        .create(
            private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    // Nothing is supplied, so not even the Wallet Key is needed.
    d.update(&p, set_hidden(&[(field::KEY, "  \n")]), None, T0 + 1)
        .unwrap();
    assert_eq!(
        d.reveal_wallet(&p, field::KEY, &CIPHER).unwrap().as_str(),
        "fake-private-key-1"
    );

    // An explicitly empty value still clears.
    d.update(&a, set_hidden(&[(field::SECRET, "")]), None, T0 + 2)
        .unwrap();
    assert_eq!(d.reveal(&a, field::SECRET), Err(NotesError::EmptyField));
    assert_eq!(
        d.reveal(&a, field::KEY).unwrap().as_str(),
        "fake-api-key-0000"
    );
    d.update(&p, set_hidden(&[(field::KEY, "")]), Some(&CIPHER), T0 + 2)
        .unwrap();
    assert_eq!(
        d.reveal_wallet(&p, field::KEY, &CIPHER),
        Err(NotesError::EmptyField)
    );
}

#[test]
fn passwords_passphrases_and_text_bodies_are_never_trimmed() {
    let mut d = doc();
    for password in ["  fake pw with spaces  ", "   ", "fake-pw\n", "\tfake-pw"] {
        let l = d
            .create(login("L", "site.example.test", "me", password), None, T0)
            .unwrap();
        assert_eq!(d.reveal(&l, field::PASSWORD).unwrap().as_str(), password);
    }
    let body = "  fake body, indented\n\n";
    let t = d.create(text("T", body), None, T0).unwrap();
    assert_eq!(d.reveal(&t, field::BODY).unwrap().as_str(), body);
    d.update(&t, set_hidden(&[(field::BODY, " \n ")]), None, T0)
        .unwrap();
    assert_eq!(d.reveal(&t, field::BODY).unwrap().as_str(), " \n ");

    let mut wallet = seed("S", "W", SEED_12);
    wallet.hidden.insert(
        field::PASSPHRASE.into(),
        zeroize::Zeroizing::new(" fake extra word ".into()),
    );
    let s = d.create(wallet, Some(&CIPHER), T0).unwrap();
    assert_eq!(
        d.reveal_wallet(&s, field::PASSPHRASE, &CIPHER)
            .unwrap()
            .as_str(),
        " fake extra word "
    );
    d.update(
        &s,
        set_hidden(&[(field::PASSPHRASE, "  ")]),
        Some(&CIPHER),
        T0,
    )
    .unwrap();
    assert_eq!(
        d.reveal_wallet(&s, field::PASSPHRASE, &CIPHER)
            .unwrap()
            .as_str(),
        "  "
    );
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
fn an_edit_that_changes_nothing_is_not_a_change() {
    let mut d = doc();
    let id = d
        .create(login("L", "site.example.test", "me", "fake-pw"), None, T0)
        .unwrap();
    let p = d
        .create(
            private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    let before = d.to_json();
    let (at, n) = (d.changed_at(), d.change_counter());

    let same_everything = || NoteEdit {
        title: Some("L".into()),
        visible: map(&[
            (field::WEBSITE, "site.example.test"),
            (field::USERNAME, "me"),
        ]),
        ..Default::default()
    };
    let edits = [
        // What an edit form sends when it is saved untouched.
        same_everything(),
        // Nothing at all.
        NoteEdit::default(),
        // Only the title, or only one value, as it already is.
        NoteEdit {
            title: Some("L".into()),
            ..Default::default()
        },
        set_visible(&[(field::USERNAME, "me")]),
        // Differences that trimming takes away.
        NoteEdit {
            title: Some("  L ".into()),
            visible: map(&[(field::WEBSITE, " site.example.test\n")]),
            ..Default::default()
        },
        // Clearing a field that has no value.
        set_visible(&[(field::PURPOSE, "")]),
        set_visible(&[(field::PURPOSE, "   ")]),
    ];
    for (i, edit) in edits.into_iter().enumerate() {
        d.update(&id, edit, None, T0 + 50).unwrap();
        assert_eq!((d.changed_at(), d.change_counter()), (at, n), "edit {i}");
        assert_eq!(d.get(&id).unwrap().updated_at, T0, "edit {i}");
    }
    // A Wallet Kind too, and without a Wallet Key.
    d.update(
        &p,
        NoteEdit {
            title: Some("P".into()),
            visible: map(&[(field::CHAIN, "evm"), (field::ADDRESS, "0xFAKE")]),
            ..Default::default()
        },
        None,
        T0 + 50,
    )
    .unwrap();
    assert_eq!((d.changed_at(), d.change_counter()), (at, n));
    assert_eq!(d.get(&p).unwrap().updated_at, T0);
    // Not one byte of what would be saved is different.
    assert_eq!(*d.to_json(), *before);

    // Each real difference is a change again: a title, a value, a new field, a removed one.
    let real = [
        NoteEdit {
            title: Some("l".into()),
            ..Default::default()
        },
        set_visible(&[(field::USERNAME, "Me")]),
        set_visible(&[(field::PURPOSE, "fake purpose")]),
        set_visible(&[(field::WEBSITE, "")]),
    ];
    for (i, edit) in real.into_iter().enumerate() {
        let (n, now) = (d.change_counter(), T0 + 60 + i as i64);
        d.update(&id, edit, None, now).unwrap();
        assert_eq!(d.change_counter(), n + 1, "real edit {i}");
        assert_eq!(d.changed_at(), now, "real edit {i}");
        assert_eq!(d.get(&id).unwrap().updated_at, now, "real edit {i}");
    }
}

#[test]
fn a_supplied_hidden_field_is_a_change_even_when_the_value_is_the_same() {
    let mut d = doc();
    let l = d
        .create(login("L", "site.example.test", "me", "fake-pw"), None, T0)
        .unwrap();
    let p = d
        .create(
            private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    let n = d.change_counter();

    d.update(
        &l,
        set_hidden(&[(field::PASSWORD, "fake-pw")]),
        None,
        T0 + 7,
    )
    .unwrap();
    assert_eq!(d.change_counter(), n + 1);
    assert_eq!(d.changed_at(), T0 + 7);
    assert_eq!(d.get(&l).unwrap().updated_at, T0 + 7);
    assert_eq!(d.reveal(&l, field::PASSWORD).unwrap().as_str(), "fake-pw");

    // A Wallet Kind: the same key again is still sealed again, so it needs the Wallet Key.
    let same_key = || set_hidden(&[(field::KEY, "fake-private-key-1")]);
    assert_eq!(
        d.update(&p, same_key(), None, T0 + 8),
        Err(NotesError::NeedsWalletKey)
    );
    assert_eq!(d.change_counter(), n + 1);
    d.update(&p, same_key(), Some(&CIPHER), T0 + 8).unwrap();
    assert_eq!(d.change_counter(), n + 2);
    assert_eq!(d.get(&p).unwrap().updated_at, T0 + 8);

    // With everything else unchanged alongside it.
    let edit = NoteEdit {
        title: Some("L".into()),
        visible: map(&[(field::USERNAME, "me")]),
        hidden: hidden(&[(field::PASSWORD, "fake-pw")]),
    };
    d.update(&l, edit, None, T0 + 9).unwrap();
    assert_eq!(d.change_counter(), n + 3);
    assert_eq!(d.get(&l).unwrap().updated_at, T0 + 9);
}

#[test]
fn a_copy_is_older_when_the_counter_or_the_clock_says_the_other_has_more() {
    let mut a = Document::new(T0);
    let mut b = Document::new(T0);
    assert!(!a.is_older_than(&b) && !b.is_older_than(&a));
    // b gained a change while its clock was behind: the counter still shows a lacks it.
    b.create(text("T", "fake"), None, T0 - 100).unwrap();
    assert!(a.is_older_than(&b));
    // Both copies were then changed separately, so each may lack something the other has,
    // whichever has the higher counter.
    a.create(text("T", "fake"), None, T0 + 1).unwrap();
    b.set_favorite(&b.list(Filter::All, "")[0].id.clone(), true, T0 - 50)
        .unwrap();
    assert!(a.change_counter() < b.change_counter());
    assert!(a.is_older_than(&b));
    assert!(b.is_older_than(&a));
    // A copy that is ahead on both counts lacks nothing.
    let mut c = Document::from_json(&a.to_json()).unwrap();
    c.create(text("U", "fake"), None, T0 + 2).unwrap();
    assert!(a.is_older_than(&c));
    assert!(!c.is_older_than(&a));
}

#[test]
fn a_wrong_clock_cannot_shorten_the_30_days_in_trash() {
    // The clock was behind when the Note was trashed: the 30 days count from the last change.
    let mut d = doc();
    let id = d.create(text("T", "fake"), None, T0).unwrap();
    d.trash(&id, T0 - 5_000_000).unwrap();
    assert_eq!(
        d.list(Filter::Trash, "")[0].deleted_at,
        Some(T0),
        "deleted_at is never earlier than the Vault's last change"
    );

    // A clock that is behind the Vault's last change removes nothing, however old Trash looks.
    let mut d = doc();
    let id = d.create(text("T", "fake"), None, T0).unwrap();
    d.trash(&id, T0).unwrap();
    d.create(text("Later", "fake"), None, T0 + 2 * TRASH_RETENTION_SECS)
        .unwrap();
    assert_eq!(d.expired_count(T0 + TRASH_RETENTION_SECS), 0);
    assert_eq!(d.purge_expired(T0 + TRASH_RETENTION_SECS), 0);
    assert_eq!(d.expired_count(T0 + 2 * TRASH_RETENTION_SECS), 1);
    assert_eq!(d.purge_expired(T0 + 2 * TRASH_RETENTION_SECS), 1);
}

#[test]
fn an_unusual_seed_phrase_keeps_its_capitals() {
    let mut d = doc();
    // Not BIP39 words, so nothing is known about how they should be written.
    let typed = "  Fake  WORD one Two three four five six seven eight nine ten ";
    let id = d
        .create(seed("S", "Main", typed), Some(&CIPHER), T0)
        .unwrap();
    assert_eq!(
        d.reveal_wallet(&id, field::WORDS, &CIPHER)
            .unwrap()
            .as_str(),
        "Fake WORD one Two three four five six seven eight nine ten"
    );
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
    d.create(
        new_note(
            Kind::ApiKey,
            "Two",
            &[(field::SERVICE, "pair.example.test")],
            &[
                (field::KEY, "fake-first-marker"),
                (field::SECRET, "fake-second-marker"),
            ],
        ),
        None,
        T0,
    )
    .unwrap();
    for q in [
        "correct-horse",
        "body-marker",
        "token-marker",
        "abandon",
        "pk-marker",
        "first-marker",
        "second-marker",
    ] {
        assert!(
            d.list(Filter::All, q).is_empty(),
            "query {q:?} matched a Hidden Field"
        );
    }
    // A term found only in a Hidden Field doesn't count towards a query of several terms
    // either, even when every other term is in a visible part of the same Note.
    for q in [
        "L correct-horse",
        "site.example.test me test-password",
        "T body-marker",
        "svc token-marker A",
        "abandon W S",
        "abandon about",
        "other addr pk-marker",
        "pair.example.test second-marker",
        "Two fake-first-marker fake-second-marker",
    ] {
        assert!(
            d.list(Filter::All, q).is_empty(),
            "query {q:?} matched a Hidden Field"
        );
    }
    // The visible terms alone do find those Notes.
    assert_eq!(titles(&d.list(Filter::All, "site.example.test me")), ["L"]);
    assert_eq!(titles(&d.list(Filter::All, "other addr")), ["P"]);
    assert_eq!(
        titles(&d.list(Filter::All, "pair.example.test two")),
        ["Two"]
    );
}

#[test]
fn search_terms_may_be_in_different_fields_and_in_any_order() {
    let mut d = doc();
    d.create(
        login("Work mail", "mail.example.test", "someone", "fake-pw"),
        None,
        T0,
    )
    .unwrap();
    d.create(
        login("Home mail", "mail.example.test", "someone-else", "fake-pw"),
        None,
        T0 + 1,
    )
    .unwrap();
    d.create(
        api_key("CI", "Builds.Example.test", "fake-token"),
        None,
        T0 + 2,
    )
    .unwrap();
    // One term in the title, the other in a visible field.
    assert_eq!(titles(&d.list(Filter::All, "work example")), ["Work mail"]);
    // The other order, another case, extra whitespace of every sort.
    assert_eq!(
        titles(&d.list(Filter::All, " \tEXAMPLE \n  Work  ")),
        ["Work mail"]
    );
    // Two different visible fields, neither of them the title.
    assert_eq!(
        titles(&d.list(Filter::All, "someone-else mail.example")),
        ["Home mail"]
    );
    assert_eq!(
        titles(&d.list(Filter::All, "mail.example someone-else")),
        ["Home mail"]
    );
    // Both terms in one field still match.
    assert_eq!(titles(&d.list(Filter::All, "builds example")), ["CI"]);
    // Every term has to be found in the same Note.
    assert!(d.list(Filter::All, "work builds").is_empty());
    assert!(d.list(Filter::All, "work zzz-no-match").is_empty());
    // A query of only whitespace is an empty query.
    assert_eq!(d.list(Filter::All, " \t ").len(), 3);
}

#[test]
fn searching_with_several_terms_keeps_the_usual_order() {
    let mut d = doc();
    let old = d
        .create(
            login("Old mail", "mail.example.test", "me", "fake-pw"),
            None,
            T0,
        )
        .unwrap();
    d.create(
        login("New mail", "mail.example.test", "me", "fake-pw"),
        None,
        T0 + 1,
    )
    .unwrap();
    d.create(
        login("Mid mail", "mail.example.test", "me", "fake-pw"),
        None,
        T0,
    )
    .unwrap();
    let all = titles(&d.list(Filter::All, "")).join(",");
    assert_eq!(titles(&d.list(Filter::All, "me mail")).join(","), all);
    // Favorites first, as without a query, however well anything else matches.
    d.set_favorite(&old, true, T0 + 2).unwrap();
    let found = d.list(Filter::All, "example mail me");
    assert_eq!(titles(&found)[0], "Old mail");
    assert_eq!(found, d.list(Filter::All, ""));
}

#[test]
fn a_private_key_is_found_by_the_name_of_its_chain_in_words() {
    let mut d = doc();
    for (title, chain) in [
        ("Hot", "evm"),
        ("Sol", "solana"),
        ("Btc", "bitcoin"),
        ("Misc", "other"),
        ("Loud", " EVM "),
        ("Custom", "Fakechain"),
    ] {
        d.create(
            private_key(title, chain, "fake-address", "fake-key"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    }
    // Other Kinds have no chain, whatever their visible values say.
    d.create(api_key("Node", "evm", "fake-token"), None, T0)
        .unwrap();
    d.create(seed("Seed", "evm", SEED_12), Some(&CIPHER), T0)
        .unwrap();

    let found = |query: &str| {
        let mut found: Vec<String> = d
            .list(Filter::All, query)
            .into_iter()
            .map(|s| s.title)
            .collect();
        found.sort();
        found
    };
    assert_eq!(found("ethereum"), ["Hot", "Loud"]);
    assert_eq!(found("Ethereum hot"), ["Hot"]);
    assert_eq!(found("fake-address ETHER"), ["Hot", "Loud"]);
    // The stored short name still works, for every Kind that has it in a visible value.
    assert_eq!(found("evm"), ["Hot", "Loud", "Node", "Seed"]);
    assert_eq!(found("solana"), ["Sol"]);
    assert_eq!(found("bitcoin"), ["Btc"]);
    assert_eq!(found("fakechain"), ["Custom"]);
    assert!(found("ethereum sol").is_empty());
}

#[test]
fn search_does_not_match_the_name_of_a_kind() {
    let mut d = doc();
    d.create(text("T", "fake"), None, T0).unwrap();
    d.create(login("L", "site.example.test", "me", "fake-pw"), None, T0)
        .unwrap();
    d.create(api_key("A", "svc.example.test", "fake-token"), None, T0)
        .unwrap();
    d.create(seed("S", "W", SEED_12), Some(&CIPHER), T0)
        .unwrap();
    d.create(
        private_key("P", "other", "fake-address", "fake-key"),
        Some(&CIPHER),
        T0,
    )
    .unwrap();
    for q in [
        "text", "login", "api key", "api_key", "seed", "phrase", "private",
    ] {
        assert!(d.list(Filter::All, q).is_empty(), "{q:?} matched a Kind");
    }
}

#[test]
fn the_visible_fields_added_in_schema_2_are_searchable() {
    let mut d = doc();
    d.create(
        new_note(
            Kind::ApiKey,
            "Deploy",
            &[
                (field::SERVICE, "svc.example.test"),
                (field::KEY_ID, "FAKEKEYID0001"),
                (field::PURPOSE, "Nightly release uploads"),
                (field::URL, "https://console.example.test/keys"),
                (field::EXPIRES, "2027-01-31"),
                (field::ENV_VAR, "FAKE_DEPLOY_KEY"),
            ],
            &[(field::KEY, "fake-token")],
        ),
        None,
        T0,
    )
    .unwrap();
    let mut wallet = seed("Cold", "Main", SEED_12);
    wallet
        .visible
        .insert(field::ADDRESS.into(), "0xFAKESEEDADDR".into());
    wallet
        .visible
        .insert(field::DERIVATION_PATH.into(), "m/44'/60'/0'/0/0".into());
    d.create(wallet, Some(&CIPHER), T0).unwrap();
    for q in [
        "fakekeyid",
        "nightly uploads",
        "console.example",
        "2027-01",
        "fake_deploy_key",
        "deploy svc nightly",
    ] {
        assert_eq!(titles(&d.list(Filter::All, q)), ["Deploy"], "{q:?}");
    }
    assert_eq!(titles(&d.list(Filter::All, "fakeseedaddr")), ["Cold"]);
    assert_eq!(titles(&d.list(Filter::All, "m/44' main")), ["Cold"]);
    // A Seed Phrase's word count is still not searchable.
    assert!(d.list(Filter::All, "12").is_empty());
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

    // The same when the document is written as schema 2.
    d.create(
        new_note(
            Kind::ApiKey,
            "A",
            &[
                (field::SERVICE, "svc.example.test"),
                (field::PURPOSE, "fake purpose"),
            ],
            &[
                (field::KEY, "fake-api-key-0000"),
                (field::SECRET, "fake-api-secret-0000"),
            ],
        ),
        None,
        T0,
    )
    .unwrap();
    let out = d.to_json();
    assert_eq!(written_schema(&d), 2);
    assert_eq!(out.capacity(), out.len());
    assert!(Document::from_json(&out).is_ok());
}

/// A schema 1 body: what every Vault written before schema 2 existed holds.
fn body_with_note(note: serde_json::Value) -> Vec<u8> {
    body_as(BASE_SCHEMA_VERSION, note)
}

fn body_as(schema_version: u32, note: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": schema_version, "changed_at": T0, "change_counter": 1, "notes": [note]
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
fn from_json_preserves_escaped_hidden_field_values() {
    let note = raw_note(
        "login",
        json!({"website": "site.example.test"}),
        json!({"password": "fake\\npassword\\\\value\\\""}),
        json!({}),
    );
    let parsed = Document::from_json(&body_with_note(note)).unwrap();
    assert_eq!(parsed.count(), 1);
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
    // A newer schema is reported as such, not as damage.
    assert_eq!(
        Document::from_json(&future(login(FAKE.into()))).err(),
        Some(NotesError::UnsupportedSchema(SCHEMA_VERSION + 1))
    );
}

#[test]
fn a_change_counter_that_cannot_grow_is_rejected() {
    let body = |counter: u64| {
        serde_json::to_vec(&json!({
            "schema_version": SCHEMA_VERSION, "changed_at": T0, "change_counter": counter, "notes": []
        }))
        .unwrap()
    };
    assert!(Document::from_json(&body(notes::MAX_CHANGE_COUNTER)).is_ok());
    for counter in [notes::MAX_CHANGE_COUNTER + 1, u64::MAX] {
        let e = Document::from_json(&body(counter)).err().expect("rejected");
        assert!(matches!(e, NotesError::Corrupt(_)), "{e:?}");
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

// ---------- document schema 2: the optional fields ----------

/// Every field schema 2 added, with a fake value that keeps the field's rule.
const NEW_FIELDS: [(Kind, &str, &str); 9] = [
    (Kind::ApiKey, field::KEY_ID, "FAKEKEYID0001"),
    (Kind::ApiKey, field::PURPOSE, "Nightly release uploads"),
    (
        Kind::ApiKey,
        field::URL,
        "https://console.example.test/keys",
    ),
    (Kind::ApiKey, field::EXPIRES, "2027-01-31"),
    (Kind::ApiKey, field::ENV_VAR, "FAKE_DEPLOY_KEY"),
    (Kind::ApiKey, field::SECRET, "fake-api-secret-0000"),
    (Kind::Login, field::PURPOSE, "Shared with the fake team"),
    (Kind::SeedPhrase, field::ADDRESS, "0xFAKESEEDADDR"),
    (Kind::SeedPhrase, field::DERIVATION_PATH, "m/44'/60'/0'/0/0"),
];

/// A Note of `kind` that uses only fields schema 1 already had.
fn plain_note(kind: Kind) -> NewNote {
    match kind {
        Kind::SeedPhrase => seed("S", "Main", SEED_12),
        Kind::PrivateKey => private_key("P", "evm", "0xFAKE", "fake-private-key-1"),
        Kind::Login => login("L", "site.example.test", "me", "fake-pw"),
        Kind::ApiKey => api_key("A", "svc.example.test", "fake-token"),
        Kind::Text => text("T", "fake body"),
    }
}

/// `plain_note(kind)` with one more field, visible or hidden.
fn note_with(kind: Kind, name: &str, value: &str) -> NewNote {
    let mut new = plain_note(kind);
    if kind.hidden_fields().contains(&name) {
        new.hidden
            .insert(name.into(), zeroize::Zeroizing::new(value.into()));
    } else {
        new.visible.insert(name.into(), value.into());
    }
    new
}

/// An edit that sets one field, visible or hidden, and nothing else.
fn set_field(kind: Kind, name: &str, value: &str) -> NoteEdit {
    if kind.hidden_fields().contains(&name) {
        set_hidden(&[(name, value)])
    } else {
        set_visible(&[(name, value)])
    }
}

/// A hand-written Note of `kind` in the body's JSON shape, with one more field.
fn raw_note_with(kind: Kind, name: &str, value: &str) -> serde_json::Value {
    let (kind_name, mut visible, mut hidden_values, sealed) = match kind {
        Kind::ApiKey => (
            "api_key",
            json!({"service": "svc.example.test"}),
            json!({"key": "fake-token"}),
            json!({}),
        ),
        Kind::Login => (
            "login",
            json!({"website": "site.example.test"}),
            json!({"password": "fake-pw"}),
            json!({}),
        ),
        Kind::SeedPhrase => (
            "seed_phrase",
            json!({"wallet_name": "W", "word_count": "12"}),
            json!({}),
            json!({"words": "00ff"}),
        ),
        Kind::PrivateKey | Kind::Text => panic!("schema 2 added nothing to {kind:?}"),
    };
    if kind.hidden_fields().contains(&name) {
        hidden_values[name] = value.into();
    } else {
        visible[name] = value.into();
    }
    raw_note(kind_name, visible, hidden_values, sealed)
}

#[test]
fn a_vault_that_uses_no_new_field_is_written_as_schema_1() {
    let mut d = doc();
    assert_eq!(written_schema(&d), 1);
    for kind in Kind::ALL {
        d.create(plain_note(kind), Some(&CIPHER), T0).unwrap();
    }
    // A Private Key has had its address from the start; it is not a schema 2 field there.
    assert_eq!(d.schema_version(), BASE_SCHEMA_VERSION);
    assert_eq!(written_schema(&d), 1);
    let json = d.to_json();
    let back = Document::from_json(&json).unwrap();
    assert_eq!(*back.to_json(), *json);
}

#[test]
fn a_schema_1_body_reads_and_is_written_back_as_schema_1() {
    // A visible value with a line break in it: nothing checked that when schema 1 Vaults were
    // written, and they must keep opening.
    let note = raw_note(
        "login",
        json!({"website": "site.example.test", "username": "fake\nuser"}),
        json!({"password": "fake-pw"}),
        json!({}),
    );
    let body = body_as(1, note);
    let d = Document::from_json(&body).unwrap();
    assert_eq!(d.schema_version(), 1);
    let written: serde_json::Value = serde_json::from_slice(&d.to_json()).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(written, original);
}

#[test]
fn using_a_new_field_writes_schema_2_and_clearing_the_last_one_returns_to_schema_1() {
    for (kind, name, value) in NEW_FIELDS {
        let mut d = doc();
        d.create(plain_note(Kind::Text), None, T0).unwrap();
        let id = d.create(plain_note(kind), Some(&CIPHER), T0).unwrap();
        assert_eq!(written_schema(&d), 1, "{kind:?} {name}");

        d.update(&id, set_field(kind, name, value), None, T0 + 1)
            .unwrap();
        assert_eq!(d.schema_version(), SCHEMA_VERSION, "{kind:?} {name}");
        assert_eq!(written_schema(&d), 2, "{kind:?} {name}");
        // It reads back with the field, and is still schema 2.
        let back = Document::from_json(&d.to_json()).unwrap();
        assert_eq!(written_schema(&back), 2, "{kind:?} {name}");
        assert_eq!(back.get(&id), d.get(&id));
        assert_eq!(*back.to_json(), *d.to_json());

        d.update(&id, set_field(kind, name, ""), None, T0 + 2)
            .unwrap();
        assert_eq!(d.schema_version(), 1, "{kind:?} {name}");
        assert_eq!(written_schema(&d), 1, "{kind:?} {name}");
        assert!(Document::from_json(&d.to_json()).is_ok());

        // Created with the field: schema 2 from the first save.
        let mut d = doc();
        d.create(note_with(kind, name, value), Some(&CIPHER), T0)
            .unwrap();
        assert_eq!(written_schema(&d), 2, "{kind:?} {name}");
    }
}

#[test]
fn clearing_one_new_field_keeps_schema_2_while_another_is_still_used() {
    let mut d = doc();
    let a = d
        .create(
            note_with(Kind::ApiKey, field::PURPOSE, "fake purpose"),
            None,
            T0,
        )
        .unwrap();
    let l = d
        .create(
            note_with(Kind::Login, field::PURPOSE, "fake purpose"),
            None,
            T0,
        )
        .unwrap();
    d.update(&a, set_visible(&[(field::EXPIRES, "2027-01-31")]), None, T0)
        .unwrap();
    for (id, name) in [(&a, field::PURPOSE), (&l, field::PURPOSE)] {
        d.update(id, set_visible(&[(name, "")]), None, T0).unwrap();
        assert_eq!(written_schema(&d), 2);
    }
    d.update(&a, set_visible(&[(field::EXPIRES, " ")]), None, T0)
        .unwrap();
    assert_eq!(written_schema(&d), 1);
}

#[test]
fn the_written_schema_follows_trash_restore_and_removal() {
    let with_new_field = || note_with(Kind::ApiKey, field::KEY_ID, "FAKEKEYID0001");
    let mut d = doc();
    d.create(plain_note(Kind::Login), None, T0).unwrap();
    assert_eq!(written_schema(&d), 1);

    // A Note in Trash is still in the document, new fields and all.
    let id = d.create(with_new_field(), None, T0).unwrap();
    assert_eq!(written_schema(&d), 2);
    d.trash(&id, T0 + 1).unwrap();
    assert_eq!(written_schema(&d), 2);
    assert!(Document::from_json(&d.to_json()).is_ok());
    d.restore(&id, T0 + 2).unwrap();
    assert_eq!(written_schema(&d), 2);

    // Delete forever.
    d.trash(&id, T0 + 3).unwrap();
    d.delete_forever(&id, T0 + 4).unwrap();
    assert_eq!(written_schema(&d), 1);

    // Empty Trash.
    let id = d.create(with_new_field(), None, T0 + 5).unwrap();
    d.trash(&id, T0 + 6).unwrap();
    assert_eq!(written_schema(&d), 2);
    assert_eq!(d.empty_trash(T0 + 7), 1);
    assert_eq!(written_schema(&d), 1);

    // The purge after 30 days in Trash.
    let id = d.create(with_new_field(), None, T0 + 8).unwrap();
    d.trash(&id, T0 + 9).unwrap();
    assert_eq!(d.purge_expired(T0 + 9 + TRASH_RETENTION_SECS - 1), 0);
    assert_eq!(written_schema(&d), 2);
    assert_eq!(d.purge_expired(T0 + 9 + TRASH_RETENTION_SECS), 1);
    assert_eq!(written_schema(&d), 1);

    // Changes that touch no field leave it where it is, either way.
    let plain = d.list(Filter::All, "")[0].id.clone();
    d.set_favorite(&plain, true, T0 + 10 + TRASH_RETENTION_SECS)
        .unwrap();
    assert_eq!(written_schema(&d), 1);
    let id = d.create(with_new_field(), None, T0).unwrap();
    d.set_favorite(&id, true, T0).unwrap();
    d.reencrypt_wallet_fields(&CIPHER, &FakeCipher { key: 9 }, T0)
        .unwrap();
    assert_eq!(written_schema(&d), 2);
}

#[test]
fn a_body_that_says_schema_1_but_uses_a_new_field_is_rejected() {
    for (kind, name, value) in NEW_FIELDS {
        let note = raw_note_with(kind, name, value);
        let e = Document::from_json(&body_as(1, note.clone()))
            .err()
            .unwrap_or_else(|| panic!("schema 1 with {kind:?} {name} must be rejected"));
        assert!(matches!(e, NotesError::Corrupt(_)), "{e:?}");
        assert!(!format!("{e:?}").contains(value), "{e:?}");
        // The same body as schema 2 is what this build writes.
        let d = Document::from_json(&body_as(2, note)).unwrap();
        assert_eq!(written_schema(&d), 2, "{kind:?} {name}");
    }
}

#[test]
fn a_schema_2_body_that_uses_no_new_field_is_read_and_written_back_as_schema_1() {
    let note = raw_note(
        "login",
        json!({"website": "site.example.test"}),
        json!({"password": "fake-pw"}),
        json!({}),
    );
    let d = Document::from_json(&body_as(2, note)).unwrap();
    assert_eq!(d.count(), 1);
    assert_eq!(written_schema(&d), 1);
    let empty = serde_json::to_vec(&json!({
        "schema_version": 2, "changed_at": T0, "change_counter": 0, "notes": []
    }))
    .unwrap();
    assert_eq!(written_schema(&Document::from_json(&empty).unwrap()), 1);
}

#[test]
fn only_schema_1_and_2_are_read_and_anything_else_is_unsupported_not_damaged() {
    let body = |version: u64| {
        serde_json::to_vec(&json!({
            "schema_version": version, "changed_at": T0, "change_counter": 0, "notes": []
        }))
        .unwrap()
    };
    assert_eq!((BASE_SCHEMA_VERSION, SCHEMA_VERSION), (1, 2));
    assert!(Document::from_json(&body(1)).is_ok());
    assert!(Document::from_json(&body(2)).is_ok());
    for version in [0u32, 3, 4, 1000, u32::MAX] {
        assert_eq!(
            Document::from_json(&body(u64::from(version))).err(),
            Some(NotesError::UnsupportedSchema(version))
        );
    }
    // Whatever shape a later schema gives the rest of the body.
    assert_eq!(
        Document::from_json(br#"{"schema_version":3,"entries":{"fake":true}}"#).err(),
        Some(NotesError::UnsupportedSchema(3))
    );
    // A version that isn't a u32 at all is damage.
    for odd in [
        &br#"{"schema_version":4294967296,"notes":[]}"#[..],
        br#"{"schema_version":-1,"notes":[]}"#,
        br#"{"schema_version":"2","notes":[]}"#,
        br#"{"notes":[]}"#,
    ] {
        assert!(matches!(
            Document::from_json(odd),
            Err(NotesError::Corrupt(_))
        ));
    }
}

/// Values that break the rule for their field, by (Kind, field).
fn rule_breakers() -> Vec<(Kind, &'static str, Vec<String>)> {
    let owned = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    let long_purpose = "p".repeat(notes::MAX_PURPOSE_CHARS + 1);
    let long_variable = "V".repeat(notes::MAX_ENV_VAR_CHARS + 1);
    vec![
        (
            Kind::ApiKey,
            field::EXPIRES,
            owned(&[
                "2027-02-30",
                "2027-02-29",
                "2027-13-01",
                "2027-00-01",
                "2027-04-31",
                "31/01/2027",
                "2027-1-31",
                "27-01-31",
                "next fake year",
                "2027-01-31T00:00",
            ]),
        ),
        (
            Kind::ApiKey,
            field::ENV_VAR,
            owned(&[
                "2FAKE",
                "FAKE-KEY",
                "FAKE KEY",
                "$FAKE_KEY",
                "FAKE_KEY=1",
                &long_variable,
            ]),
        ),
        (
            Kind::ApiKey,
            field::PURPOSE,
            owned(&["two\nlines", "a\ttab", &long_purpose]),
        ),
        (
            Kind::Login,
            field::PURPOSE,
            owned(&["two\r\nlines", &long_purpose]),
        ),
        (Kind::ApiKey, field::KEY_ID, owned(&["FAKE\nKEYID"])),
        (
            Kind::ApiKey,
            field::URL,
            owned(&["https://console.example.test/\nkeys"]),
        ),
        (
            Kind::SeedPhrase,
            field::ADDRESS,
            owned(&["0xFAKE\r\nSEEDADDR"]),
        ),
        (
            Kind::SeedPhrase,
            field::DERIVATION_PATH,
            owned(&["m/44'\u{1b}[2J/0", "m/44'\u{0}"]),
        ),
    ]
}

#[test]
fn a_value_that_breaks_the_rule_for_its_field_is_refused_when_supplied() {
    for (kind, name, values) in rule_breakers() {
        for value in values {
            let mut d = doc();
            assert_eq!(
                d.create(note_with(kind, name, &value), Some(&CIPHER), T0),
                Err(NotesError::InvalidValue(name.into())),
                "{kind:?} {name} {value:?}"
            );
            assert_eq!(d.count(), 0);

            let id = d.create(plain_note(kind), Some(&CIPHER), T0).unwrap();
            let (n, title) = (d.change_counter(), d.get(&id).unwrap().title);
            let edit = NoteEdit {
                title: Some("Renamed".into()),
                visible: map(&[(name, value.as_str())]),
                ..Default::default()
            };
            assert_eq!(
                d.update(&id, edit, Some(&CIPHER), T0 + 1),
                Err(NotesError::InvalidValue(name.into())),
                "{kind:?} {name} {value:?}"
            );
            // Nothing changed, not even the title that came with it.
            assert_eq!(d.change_counter(), n);
            assert_eq!(d.get(&id).unwrap().title, title);
            assert_eq!(written_schema(&d), 1);
        }
    }
}

#[test]
fn a_body_with_a_value_that_breaks_the_rule_for_its_field_is_rejected() {
    for (kind, name, values) in rule_breakers() {
        for value in values {
            let body = body_as(2, raw_note_with(kind, name, &value));
            let e = Document::from_json(&body)
                .err()
                .unwrap_or_else(|| panic!("{kind:?} {name} {value:?} must be rejected"));
            assert!(matches!(e, NotesError::Corrupt(_)), "{e:?}");
            // The message names the field and never repeats the value.
            assert!(format!("{e:?}").contains(name), "{e:?}");
            assert!(!format!("{e:?}").contains(value.as_str()), "{e:?}");
        }
    }
    // A body is not trimmed on the way in: what the app writes is already trimmed.
    let padded = raw_note_with(Kind::ApiKey, field::EXPIRES, " 2027-01-31");
    assert!(matches!(
        Document::from_json(&body_as(2, padded)),
        Err(NotesError::Corrupt(_))
    ));
}

#[test]
fn values_at_the_edge_of_their_rule_are_accepted() {
    // 200 characters, not 200 bytes.
    let purpose = "\u{e9}".repeat(notes::MAX_PURPOSE_CHARS);
    let variable = format!("_{}", "v".repeat(notes::MAX_ENV_VAR_CHARS - 1));
    let mut d = doc();
    let a = d
        .create(
            new_note(
                Kind::ApiKey,
                "A",
                &[
                    (field::SERVICE, "svc.example.test"),
                    (field::PURPOSE, purpose.as_str()),
                    (field::ENV_VAR, variable.as_str()),
                    // A leap day, and whitespace around it that trimming takes away.
                    (field::EXPIRES, "  2028-02-29\n"),
                    (field::KEY_ID, "FAKE KEY ID with spaces"),
                    (field::URL, "console.example.test/keys?fake=1"),
                ],
                &[(field::KEY, "fake-token")],
            ),
            None,
            T0,
        )
        .unwrap();
    let view = d.get(&a).unwrap();
    assert_eq!(view.visible[field::EXPIRES], "2028-02-29");
    assert_eq!(view.visible[field::PURPOSE], purpose);
    assert_eq!(view.visible[field::ENV_VAR], variable);
    let l = d
        .create(note_with(Kind::Login, field::PURPOSE, &purpose), None, T0)
        .unwrap();
    assert_eq!(d.get(&l).unwrap().visible[field::PURPOSE], purpose);
    let back = Document::from_json(&d.to_json()).unwrap();
    assert_eq!(back.get(&a), d.get(&a));
    assert_eq!(back.get(&l), d.get(&l));
}

#[test]
fn the_new_fields_belong_to_their_kinds_only() {
    let mut d = doc();
    let wrong: [(Kind, &str); 7] = [
        (Kind::Login, field::EXPIRES),
        (Kind::Login, field::KEY_ID),
        (Kind::Text, field::PURPOSE),
        (Kind::PrivateKey, field::DERIVATION_PATH),
        (Kind::PrivateKey, field::PURPOSE),
        (Kind::SeedPhrase, field::URL),
        (Kind::ApiKey, field::DERIVATION_PATH),
    ];
    for (kind, name) in wrong {
        let mut new = plain_note(kind);
        new.visible.insert(name.into(), "fake value".into());
        assert_eq!(
            d.create(new, Some(&CIPHER), T0),
            Err(NotesError::UnknownField(name.into())),
            "{kind:?} {name}"
        );
    }
    // The second Hidden Field is the API Key's alone.
    for kind in [Kind::Login, Kind::Text, Kind::PrivateKey, Kind::SeedPhrase] {
        let mut new = plain_note(kind);
        new.hidden.insert(
            field::SECRET.into(),
            zeroize::Zeroizing::new("fake-api-secret-0000".into()),
        );
        assert_eq!(
            d.create(new, Some(&CIPHER), T0),
            Err(NotesError::UnknownField(field::SECRET.into())),
            "{kind:?}"
        );
    }
    assert_eq!(d.count(), 0);
}

#[test]
fn the_secret_of_an_api_key_is_set_shown_and_cleared_like_any_hidden_field() {
    const SECRET: &str = "fake-api-secret-0000";
    let mut d = doc();
    let id = d
        .create(
            api_key("A", "svc.example.test", "fake-api-key-0000"),
            None,
            T0,
        )
        .unwrap();
    let has = |d: &Document| d.get(&id).unwrap().hidden;
    assert_eq!(
        has(&d),
        vec![
            (field::KEY.to_string(), true),
            (field::SECRET.to_string(), false)
        ]
    );
    assert_eq!(d.reveal(&id, field::SECRET), Err(NotesError::EmptyField));

    // Set: an ordinary Hidden Field, so no Wallet Key is involved.
    d.update(&id, set_hidden(&[(field::SECRET, SECRET)]), None, T0 + 1)
        .unwrap();
    assert_eq!(
        has(&d),
        vec![
            (field::KEY.to_string(), true),
            (field::SECRET.to_string(), true)
        ]
    );
    assert_eq!(d.reveal(&id, field::SECRET).unwrap().as_str(), SECRET);
    assert_eq!(
        d.reveal(&id, field::KEY).unwrap().as_str(),
        "fake-api-key-0000"
    );
    assert!(d.reveal_wallet(&id, field::SECRET, &CIPHER).is_err());

    // It is in no list row and no view.
    let shown = format!("{:?} {:?}", d.list(Filter::All, ""), d.get(&id));
    assert!(!shown.contains(SECRET));
    // It is stored beside the key, as an ordinary Hidden Field, and survives a round trip.
    let json: serde_json::Value = serde_json::from_slice(&d.to_json()).unwrap();
    assert_eq!(json["notes"][0]["hidden"]["secret"], SECRET);
    assert_eq!(json["notes"][0]["sealed"], json!({}));
    let back = Document::from_json(&d.to_json()).unwrap();
    assert_eq!(back.reveal(&id, field::SECRET).unwrap().as_str(), SECRET);

    // Replace, then clear; the key is untouched throughout.
    d.update(
        &id,
        set_hidden(&[(field::SECRET, "fake-api-secret-1111")]),
        None,
        T0 + 2,
    )
    .unwrap();
    assert_eq!(
        d.reveal(&id, field::SECRET).unwrap().as_str(),
        "fake-api-secret-1111"
    );
    d.update(&id, set_hidden(&[(field::SECRET, "")]), None, T0 + 3)
        .unwrap();
    assert_eq!(d.reveal(&id, field::SECRET), Err(NotesError::EmptyField));
    assert_eq!(
        d.reveal(&id, field::KEY).unwrap().as_str(),
        "fake-api-key-0000"
    );
    assert_eq!(written_schema(&d), 1);
}

#[test]
fn a_seed_phrase_row_shows_its_wallet_name_and_never_its_address() {
    let mut d = doc();
    let named = d
        .create(
            note_with(Kind::SeedPhrase, field::ADDRESS, "0xFAKESEEDADDR"),
            Some(&CIPHER),
            T0,
        )
        .unwrap();
    let mut unnamed = seed("Unnamed", "", SEED_12);
    unnamed
        .visible
        .insert(field::ADDRESS.into(), "0xFAKESEEDADDR".into());
    unnamed
        .visible
        .insert(field::DERIVATION_PATH.into(), "m/44'/60'/0'/0/0".into());
    let unnamed = d.create(unnamed, Some(&CIPHER), T0).unwrap();
    let key = d
        .create(plain_note(Kind::PrivateKey), Some(&CIPHER), T0)
        .unwrap();

    let subtitle = |id: &str| {
        let rows = d.list(Filter::All, "");
        rows.iter()
            .find(|row| row.id == id)
            .unwrap()
            .subtitle
            .clone()
    };
    assert_eq!(subtitle(&named).as_deref(), Some("Main"));
    assert_eq!(subtitle(&unnamed), None);
    // A Private Key row still shows its address.
    assert_eq!(subtitle(&key).as_deref(), Some("0xFAKE"));
    // The detail view has the address all the same.
    assert_eq!(
        d.get(&unnamed).unwrap().visible[field::ADDRESS],
        "0xFAKESEEDADDR"
    );
}

#[test]
fn a_list_row_says_when_an_api_key_expires_and_whether_there_is_something_to_copy() {
    let mut d = doc();
    let mut row_of = |new: NewNote| {
        let id = d.create(new, Some(&CIPHER), T0).unwrap();
        let rows = d.list(Filter::All, "");
        rows.into_iter().find(|row| row.id == id).unwrap()
    };

    let dated = row_of(note_with(Kind::ApiKey, field::EXPIRES, "2027-01-31"));
    assert_eq!(dated.expires.as_deref(), Some("2027-01-31"));
    assert!(dated.has_hidden);
    assert_eq!(dated.subtitle.as_deref(), Some("svc.example.test"));
    let undated = row_of(plain_note(Kind::ApiKey));
    assert_eq!(undated.expires, None);

    // "Something to copy" is the Kind's first Hidden Field: the key, not the secret.
    let secret_only = row_of(new_note(
        Kind::ApiKey,
        "Secret only",
        &[(field::SERVICE, "svc.example.test")],
        &[(field::SECRET, "fake-api-secret-0000")],
    ));
    assert!(!secret_only.has_hidden);

    for kind in [Kind::SeedPhrase, Kind::PrivateKey, Kind::Login, Kind::Text] {
        let row = row_of(plain_note(kind));
        assert_eq!(row.expires, None, "{kind:?}");
        assert!(row.has_hidden, "{kind:?}");
    }
    for kind in [Kind::PrivateKey, Kind::Login, Kind::Text] {
        let row = row_of(new_note(kind, "Empty", &[], &[]));
        assert!(!row.has_hidden, "{kind:?}");
    }
    // A Seed Phrase with a Passphrase and one without: both have their words.
    let row = row_of(note_with(
        Kind::SeedPhrase,
        field::PASSPHRASE,
        "fake extra word",
    ));
    assert!(row.has_hidden);
}

// ---------- adding to a Text body ----------

#[test]
fn appending_to_a_text_body_adds_a_line_without_resupplying_the_rest() {
    let mut d = doc();
    let id = d.create(text("T", "fake line one"), None, T0).unwrap();
    let n = d.change_counter();
    d.append_text(&id, "fake line two", T0 + 5).unwrap();
    assert_eq!(
        d.reveal(&id, field::BODY).unwrap().as_str(),
        "fake line one\nfake line two"
    );
    // It counts as a change.
    assert_eq!(d.change_counter(), n + 1);
    assert_eq!(d.changed_at(), T0 + 5);
    assert_eq!(d.get(&id).unwrap().updated_at, T0 + 5);
    assert_eq!(d.get(&id).unwrap().created_at, T0);

    // A body that already ends with a line break gets no second one.
    d.append_text(&id, "fake line three\n", T0 + 6).unwrap();
    d.append_text(&id, "fake line four\r\n", T0 + 7).unwrap();
    d.append_text(&id, "  fake line five, as typed \n\n", T0 + 8)
        .unwrap();
    d.append_text(&id, " ", T0 + 9).unwrap();
    assert_eq!(
        d.reveal(&id, field::BODY).unwrap().as_str(),
        "fake line one\nfake line two\nfake line three\nfake line four\r\n  fake line five, as typed \n\n "
    );
    assert_eq!(d.change_counter(), n + 5);
    // The longer body is what is saved and read back.
    let back = Document::from_json(&d.to_json()).unwrap();
    assert_eq!(back.reveal(&id, field::BODY), d.reveal(&id, field::BODY));
    assert_eq!(written_schema(&d), 1);
}

#[test]
fn appending_to_an_empty_text_body_starts_it_without_a_blank_line() {
    let mut d = doc();
    let id = d
        .create(new_note(Kind::Text, "Empty", &[], &[]), None, T0)
        .unwrap();
    assert_eq!(d.reveal(&id, field::BODY), Err(NotesError::EmptyField));
    d.append_text(&id, "fake first line", T0 + 1).unwrap();
    assert_eq!(
        d.reveal(&id, field::BODY).unwrap().as_str(),
        "fake first line"
    );
    assert_eq!(
        d.get(&id).unwrap().hidden,
        vec![(field::BODY.to_string(), true)]
    );
}

#[test]
fn appending_nothing_or_to_another_kind_is_refused_and_changes_nothing() {
    let mut d = doc();
    let t = d.create(text("T", "fake line one"), None, T0).unwrap();
    let others: Vec<String> = [
        Kind::SeedPhrase,
        Kind::PrivateKey,
        Kind::Login,
        Kind::ApiKey,
    ]
    .into_iter()
    .map(|kind| d.create(plain_note(kind), Some(&CIPHER), T0).unwrap())
    .collect();
    let (at, n) = (d.changed_at(), d.change_counter());
    let before = d.to_json();

    assert_eq!(
        d.append_text(&t, "", T0 + 9),
        Err(NotesError::InvalidValue(field::BODY.into()))
    );
    for id in &others {
        assert_eq!(
            d.append_text(id, "fake line", T0 + 9),
            Err(NotesError::UnknownField(field::BODY.into()))
        );
    }
    assert_eq!(
        d.append_text("nope", "fake line", T0 + 9),
        Err(NotesError::NotFound)
    );
    assert_eq!((d.changed_at(), d.change_counter()), (at, n));
    assert_eq!(*d.to_json(), *before);
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
