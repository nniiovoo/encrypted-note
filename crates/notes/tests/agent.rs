//! The "added by an agent" mark (ADR-0012, document schema 3) and the agent's view of the
//! Vault. Fake values only.

mod common;

use std::collections::BTreeMap;

use common::*;
use notes::{
    AGENT_MARK_SCHEMA_VERSION, AgentEditRule, Document, Filter, Kind, NewNote, NoteEdit,
    NotesError, TRASH_RETENTION_SECS, agent_edit_needs, agent_edit_rule, field,
};
use serde_json::json;

const T0: i64 = 1_700_000_000;

/// Every Hidden Field value the fixtures below store. None may reach an agent.
const FAKE_HIDDEN: [&str; 7] = [
    "fake-login-password-0001",
    "fake-api-key-0001",
    "fake-api-secret-0001",
    "fake-text-body-0001",
    "fake-private-key-0001",
    "fake-passphrase-0001",
    "abandon",
];

/// Field names and fake values.
type Pairs = &'static [(&'static str, &'static str)];

/// A Note of `kind` with every visible and Hidden Field it has set to a fake value.
fn full_note(kind: Kind, title: &str) -> NewNote {
    let (visible, hidden_values): (Pairs, Pairs) = match kind {
        Kind::SeedPhrase => (
            &[
                (field::WALLET_NAME, "Fake wallet"),
                (field::ADDRESS, "0x000000000000000000000000000000000000fa01"),
                (field::DERIVATION_PATH, "m/44'/60'/0'/0/0"),
            ],
            &[
                (field::WORDS, SEED_12),
                (field::PASSPHRASE, "fake-passphrase-0001"),
            ],
        ),
        Kind::PrivateKey => (
            &[
                (field::CHAIN, "evm"),
                (field::ADDRESS, "0x000000000000000000000000000000000000fa02"),
            ],
            &[(field::KEY, "fake-private-key-0001")],
        ),
        Kind::Login => (
            &[
                (field::WEBSITE, "site.example.test"),
                (field::USERNAME, "fake-user"),
                (field::PURPOSE, "Fake purpose"),
            ],
            &[(field::PASSWORD, "fake-login-password-0001")],
        ),
        Kind::ApiKey => (
            &[
                (field::SERVICE, "svc.example.test"),
                (field::KEY_ID, "FAKEKEYID0001"),
                (field::PURPOSE, "Fake uploads"),
                (field::URL, "https://console.example.test/keys"),
                (field::EXPIRES, "2027-01-31"),
                (field::ENV_VAR, "FAKE_DEPLOY_KEY"),
            ],
            &[
                (field::KEY, "fake-api-key-0001"),
                (field::SECRET, "fake-api-secret-0001"),
            ],
        ),
        Kind::Text => (&[], &[(field::BODY, "fake-text-body-0001")]),
    };
    NewNote {
        kind,
        title: title.into(),
        visible: map(visible),
        hidden: hidden(hidden_values),
    }
}

/// A Note of `kind` that uses only fields schema 1 already had.
fn plain_note(kind: Kind) -> NewNote {
    match kind {
        Kind::SeedPhrase => seed("S", "Main", SEED_12),
        Kind::PrivateKey => private_key("P", "evm", "0xFAKE", "fake-private-key-0001"),
        Kind::Login => login("L", "site.example.test", "me", "fake-login-password-0001"),
        Kind::ApiKey => api_key("A", "svc.example.test", "fake-api-key-0001"),
        Kind::Text => text("T", "fake-text-body-0001"),
    }
}

fn cipher_for(kind: Kind) -> Option<&'static dyn notes::WalletCipher> {
    kind.is_wallet()
        .then_some(&CIPHER as &dyn notes::WalletCipher)
}

/// The `schema_version` a document is written with.
fn written_schema(d: &Document) -> u64 {
    let json: serde_json::Value = serde_json::from_slice(&d.to_json()).unwrap();
    json["schema_version"].as_u64().unwrap()
}

/// The written JSON of the only Note in `d`.
fn written_note(d: &Document) -> serde_json::Value {
    let json: serde_json::Value = serde_json::from_slice(&d.to_json()).unwrap();
    json["notes"][0].clone()
}

fn raw_login(mark: Option<bool>) -> serde_json::Value {
    let mut note = json!({
        "id": "00112233445566778899aabbccddeeff", "kind": "login", "title": "T",
        "visible": {"website": "site.example.test"},
        "hidden": {"password": "fake-login-password-0001"}, "sealed": {},
        "favorite": false, "created_at": T0, "updated_at": T0, "deleted_at": null
    });
    if let Some(mark) = mark {
        note["added_by_agent"] = mark.into();
    }
    note
}

fn body_as(schema_version: u32, notes: &[serde_json::Value]) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema_version": schema_version, "changed_at": T0, "change_counter": 1, "notes": notes
    }))
    .unwrap()
}

// ---------- the mark ----------

#[test]
fn create_never_marks_and_create_by_agent_always_does() {
    for kind in Kind::ALL {
        let mut d = Document::new(T0);
        let owner = d.create(plain_note(kind), cipher_for(kind), T0).unwrap();
        let agent = d
            .create_by_agent(plain_note(kind), cipher_for(kind), T0)
            .unwrap();
        assert!(!d.get(&owner).unwrap().added_by_agent, "{kind:?}");
        assert!(d.get(&agent).unwrap().added_by_agent, "{kind:?}");
        let rows = d.list(Filter::All, "");
        let row = |id: &str| rows.iter().find(|r| r.id == id).unwrap().added_by_agent;
        assert!(!row(&owner), "{kind:?}");
        assert!(row(&agent), "{kind:?}");
        assert_eq!(d.change_counter(), 2);
    }
}

#[test]
fn create_by_agent_checks_a_new_note_exactly_like_create() {
    for kind in [Kind::SeedPhrase, Kind::PrivateKey] {
        let mut d = Document::new(T0);
        assert_eq!(
            d.create_by_agent(plain_note(kind), None, T0),
            Err(NotesError::NeedsWalletKey),
            "{kind:?}"
        );
        assert_eq!(d.count(), 0);
        assert_eq!(d.change_counter(), 0);
        let id = d
            .create_by_agent(full_note(kind, "Fake wallet"), Some(&CIPHER), T0)
            .unwrap();
        // The words were sealed with the Wallet Key, as `create` does.
        assert!(!String::from_utf8_lossy(&d.to_json()).contains("abandon"));
        let words = d.reveal_wallet(&id, kind.hidden_fields()[0], &CIPHER);
        assert!(words.is_ok(), "{kind:?}");
    }
    let mut d = Document::new(T0);
    assert_eq!(
        d.create_by_agent(seed("S", "W", "abandon about"), Some(&CIPHER), T0),
        Err(NotesError::BadWordCount(2))
    );
    assert_eq!(
        d.create_by_agent(text("  ", "fake-text-body-0001"), None, T0),
        Err(NotesError::EmptyTitle)
    );
    let mut bad = plain_note(Kind::Login);
    bad.visible.insert("wallet_name".into(), "x".into());
    assert_eq!(
        d.create_by_agent(bad, None, T0),
        Err(NotesError::UnknownField("wallet_name".into()))
    );
    assert_eq!(d.count(), 0);
}

#[test]
fn owner_changes_keep_the_mark() {
    let mut d = Document::new(T0);
    let id = d
        .create_by_agent(plain_note(Kind::Login), None, T0)
        .unwrap();
    let edit = NoteEdit {
        title: Some("Renamed by the owner".into()),
        visible: map(&[(field::PURPOSE, "Owner's purpose")]),
        hidden: hidden(&[(field::PASSWORD, "fake-login-password-0002")]),
    };
    d.update(&id, edit, None, T0 + 1).unwrap();
    d.set_favorite(&id, true, T0 + 2).unwrap();
    d.trash(&id, T0 + 4).unwrap();
    assert!(d.get(&id).unwrap().added_by_agent);
    d.restore(&id, T0 + 5).unwrap();
    assert!(d.get(&id).unwrap().added_by_agent);

    let w = d
        .create_by_agent(plain_note(Kind::PrivateKey), Some(&CIPHER), T0)
        .unwrap();
    d.reencrypt_wallet_fields(&CIPHER, &FakeCipher { key: 9 }, T0 + 6)
        .unwrap();
    assert!(d.get(&w).unwrap().added_by_agent);
    let back = Document::from_json(&d.to_json()).unwrap();
    assert!(back.get(&id).unwrap().added_by_agent);
    assert!(back.get(&w).unwrap().added_by_agent);
}

#[test]
fn clear_agent_mark_is_the_owners_change_and_leaves_the_note_as_it_was() {
    let mut d = Document::new(T0);
    let id = d
        .create_by_agent(plain_note(Kind::ApiKey), None, T0)
        .unwrap();
    let owner = d.create(plain_note(Kind::Text), None, T0).unwrap();
    let before = d.get(&id).unwrap();
    let counter = d.change_counter();

    d.clear_agent_mark(&id, T0 + 10).unwrap();
    let after = d.get(&id).unwrap();
    assert!(!after.added_by_agent);
    assert_eq!(after.updated_at, before.updated_at);
    assert_eq!(after.visible, before.visible);
    assert_eq!(d.change_counter(), counter + 1);
    assert_eq!(d.changed_at(), T0 + 10);
    assert!(d.agent_note(&id).is_none());

    // No mark: nothing changes.
    for unmarked in [&id, &owner] {
        d.clear_agent_mark(unmarked, T0 + 20).unwrap();
        assert_eq!(d.change_counter(), counter + 1);
        assert_eq!(d.changed_at(), T0 + 10);
    }
    assert_eq!(
        d.clear_agent_mark("00000000000000000000000000000000", T0),
        Err(NotesError::NotFound)
    );

    // A Note in Trash can lose its mark too.
    let trashed = d.create_by_agent(plain_note(Kind::Text), None, T0).unwrap();
    d.trash(&trashed, T0 + 30).unwrap();
    d.clear_agent_mark(&trashed, T0 + 31).unwrap();
    assert!(!d.get(&trashed).unwrap().added_by_agent);
}

// ---------- document schema 3 ----------

#[test]
fn the_mark_is_written_last_and_only_when_true() {
    let mut d = Document::new(T0);
    d.create(plain_note(Kind::Login), None, T0).unwrap();
    let unmarked = String::from_utf8(d.to_json().to_vec()).unwrap();
    assert!(!unmarked.contains("added_by_agent"), "{unmarked}");

    let mut d = Document::new(T0);
    d.create_by_agent(plain_note(Kind::Login), None, T0)
        .unwrap();
    let marked = String::from_utf8(d.to_json().to_vec()).unwrap();
    assert!(
        marked.contains(r#""deleted_at":null,"added_by_agent":true}"#),
        "{marked}"
    );
    assert_eq!(written_note(&d)["added_by_agent"], true);
}

#[test]
fn a_marked_note_writes_schema_3_and_reads_back_the_same() {
    for kind in Kind::ALL {
        let mut d = Document::new(T0);
        d.create(plain_note(kind), cipher_for(kind), T0).unwrap();
        assert_eq!(written_schema(&d), 1, "{kind:?}");
        let id = d
            .create_by_agent(plain_note(kind), cipher_for(kind), T0)
            .unwrap();
        assert_eq!(d.schema_version(), AGENT_MARK_SCHEMA_VERSION, "{kind:?}");
        assert_eq!(written_schema(&d), 3, "{kind:?}");

        let back = Document::from_json(&d.to_json()).unwrap();
        assert_eq!(written_schema(&back), 3, "{kind:?}");
        assert_eq!(back.get(&id), d.get(&id), "{kind:?}");
        assert_eq!(back.agent_notes(), d.agent_notes(), "{kind:?}");
        assert_eq!(*back.to_json(), *d.to_json(), "{kind:?}");
    }
    // Schema 3 holds schema 2's fields too.
    let mut d = Document::new(T0);
    d.create_by_agent(full_note(Kind::ApiKey, "A"), None, T0)
        .unwrap();
    assert_eq!(written_schema(&d), 3);
    let back = Document::from_json(&d.to_json()).unwrap();
    assert_eq!(*back.to_json(), *d.to_json());
}

#[test]
fn removing_the_last_mark_writes_the_older_schema_again() {
    // Back to 1 when nothing else needs more, to 2 while a schema 2 field is still used.
    for (owner_note, expected) in [
        (plain_note(Kind::Login), 1),
        (full_note(Kind::ApiKey, "A"), 2),
    ] {
        let mut d = Document::new(T0);
        d.create(owner_note, None, T0).unwrap();
        let id = d.create_by_agent(plain_note(Kind::Text), None, T0).unwrap();
        assert_eq!(written_schema(&d), 3);
        d.clear_agent_mark(&id, T0 + 1).unwrap();
        assert_eq!(written_schema(&d), expected);
        assert!(Document::from_json(&d.to_json()).is_ok());
    }
}

#[test]
fn the_written_schema_follows_trash_and_removal_of_marked_notes() {
    let mut d = Document::new(T0);
    d.create(plain_note(Kind::Login), None, T0).unwrap();

    // A marked Note in Trash is still in the document, mark and all.
    let id = d.create_by_agent(plain_note(Kind::Text), None, T0).unwrap();
    d.trash(&id, T0 + 1).unwrap();
    assert_eq!(written_schema(&d), 3);
    let back = Document::from_json(&d.to_json()).unwrap();
    assert!(back.get(&id).unwrap().added_by_agent);
    d.delete_forever(&id, T0 + 2).unwrap();
    assert_eq!(written_schema(&d), 1);

    let id = d
        .create_by_agent(plain_note(Kind::Text), None, T0 + 3)
        .unwrap();
    d.trash(&id, T0 + 4).unwrap();
    assert_eq!(d.empty_trash(T0 + 5), 1);
    assert_eq!(written_schema(&d), 1);

    let id = d
        .create_by_agent(plain_note(Kind::Text), None, T0 + 6)
        .unwrap();
    d.trash(&id, T0 + 7).unwrap();
    assert_eq!(d.purge_expired(T0 + 7 + TRASH_RETENTION_SECS - 1), 0);
    assert_eq!(written_schema(&d), 3);
    assert_eq!(d.purge_expired(T0 + 7 + TRASH_RETENTION_SECS), 1);
    assert_eq!(written_schema(&d), 1);
}

#[test]
fn a_body_that_says_schema_1_or_2_with_the_mark_is_damaged() {
    for version in [1, 2] {
        let e = Document::from_json(&body_as(version, &[raw_login(Some(true))]))
            .err()
            .unwrap_or_else(|| panic!("schema {version} with the mark must be rejected"));
        assert!(matches!(e, NotesError::Corrupt(_)), "{e:?}");
        assert!(!format!("{e:?}").contains("fake-"), "{e:?}");
    }
    let d = Document::from_json(&body_as(3, &[raw_login(Some(true))])).unwrap();
    assert_eq!(written_schema(&d), 3);
    assert_eq!(d.agent_notes().len(), 1);
}

#[test]
fn a_schema_3_body_without_the_mark_is_written_back_as_the_older_schema() {
    for mark in [None, Some(false)] {
        let d = Document::from_json(&body_as(3, &[raw_login(mark)])).unwrap();
        assert_eq!(d.count(), 1);
        assert!(d.agent_notes().is_empty());
        assert_eq!(written_schema(&d), 1);
        assert!(written_note(&d).get("added_by_agent").is_none());
    }
    // A mark of false is no mark, under every schema.
    for version in [1, 2] {
        let d = Document::from_json(&body_as(version, &[raw_login(Some(false))])).unwrap();
        assert!(!d.list(Filter::All, "")[0].added_by_agent);
    }
    // A mark that isn't a boolean is damage.
    let mut odd = raw_login(None);
    odd["added_by_agent"] = "yes".into();
    let e = Document::from_json(&body_as(3, &[odd])).err().unwrap();
    assert!(matches!(e, NotesError::Corrupt(_)), "{e:?}");
}

// ---------- what an agent sees ----------

#[test]
fn agent_notes_holds_only_marked_notes_outside_trash_newest_first() {
    let mut d = Document::new(T0);
    let owner = d.create(plain_note(Kind::Login), None, T0 + 50).unwrap();
    let older = d
        .create_by_agent(plain_note(Kind::ApiKey), None, T0)
        .unwrap();
    let newer = d
        .create_by_agent(plain_note(Kind::Text), None, T0 + 1)
        .unwrap();
    let trashed = d
        .create_by_agent(plain_note(Kind::Login), None, T0 + 2)
        .unwrap();
    d.trash(&trashed, T0 + 3).unwrap();
    // A star doesn't move a Note up the agent's list.
    d.set_favorite(&older, true, T0 + 4).unwrap();

    let ids: Vec<String> = d.agent_notes().into_iter().map(|n| n.id).collect();
    assert_eq!(ids, [newer.clone(), older.clone()]);

    // Unmarked, in Trash and missing look the same.
    for id in [
        owner.as_str(),
        &trashed,
        "00000000000000000000000000000000",
        "",
    ] {
        assert_eq!(d.agent_note(id), None, "{id}");
    }
    assert_eq!(d.agent_note(&older).unwrap().kind, Kind::ApiKey);

    // Restored from Trash, it is the agent's again; with the mark removed, it isn't.
    d.restore(&trashed, T0 + 5).unwrap();
    assert!(d.agent_note(&trashed).is_some());
    d.clear_agent_mark(&newer, T0 + 6).unwrap();
    let ids: Vec<String> = d.agent_notes().into_iter().map(|n| n.id).collect();
    assert_eq!(ids, [trashed, older]);
}

#[test]
fn an_agent_note_holds_visible_parts_only_and_no_sign_of_a_value() {
    for kind in Kind::ALL {
        let mut d = Document::new(T0);
        let id = d
            .create_by_agent(full_note(kind, "Fake title"), cipher_for(kind), T0)
            .unwrap();
        d.set_favorite(&id, true, T0).unwrap();
        let note = d.agent_note(&id).unwrap();
        assert_eq!(d.agent_notes(), std::slice::from_ref(&note));

        let json = serde_json::to_value(&note).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            ["created_at", "id", "kind", "title", "updated_at", "visible"],
            "{kind:?}"
        );
        let text = json.to_string();
        for value in FAKE_HIDDEN {
            assert!(!text.contains(value), "{kind:?} leaked {value}: {text}");
        }
        for name in kind.hidden_fields() {
            assert!(!text.contains(&format!("\"{name}\"")), "{kind:?}: {text}");
        }
        for word in [
            "favorite",
            "hidden",
            "has_",
            "word_count",
            "deleted_at",
            "added_by_agent",
        ] {
            assert!(!text.contains(word), "{kind:?} has {word}: {text}");
        }
        // Every visible value the Note has, except the word count, is there.
        let expected: BTreeMap<String, String> = d
            .get(&id)
            .unwrap()
            .visible
            .into_iter()
            .filter(|(name, _)| name != field::WORD_COUNT)
            .collect();
        assert_eq!(note.visible, expected, "{kind:?}");
        assert_eq!(json["kind"], serde_json::to_value(kind).unwrap());
    }
}

#[test]
fn an_agent_cant_tell_whether_a_seed_phrase_has_a_passphrase_or_how_many_words() {
    let mut d = Document::new(T0);
    let with = d
        .create_by_agent(full_note(Kind::SeedPhrase, "W"), Some(&CIPHER), T0)
        .unwrap();
    let mut no_passphrase = full_note(Kind::SeedPhrase, "W");
    no_passphrase.hidden.remove(field::PASSPHRASE);
    no_passphrase
        .hidden
        .insert(field::WORDS.into(), zeroize::Zeroizing::new(SEED_24.into()));
    let without = d.create_by_agent(no_passphrase, Some(&CIPHER), T0).unwrap();
    let mut a = d.agent_note(&with).unwrap();
    let b = d.agent_note(&without).unwrap();
    a.id = b.id.clone();
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

// ---------- edit rules ----------

#[test]
fn every_field_of_every_kind_has_the_agreed_edit_rule() {
    use AgentEditRule::{AtOnce, Never, OwnerConfirms, OwnerConfirmsWithPassword as Password};
    let table: &[(Kind, &str, AgentEditRule)] = &[
        (Kind::SeedPhrase, "title", Password),
        (Kind::SeedPhrase, field::WALLET_NAME, Password),
        (Kind::SeedPhrase, field::WORD_COUNT, Never),
        (Kind::SeedPhrase, field::ADDRESS, Password),
        (Kind::SeedPhrase, field::DERIVATION_PATH, Password),
        (Kind::PrivateKey, "title", Password),
        (Kind::PrivateKey, field::CHAIN, Password),
        (Kind::PrivateKey, field::ADDRESS, Password),
        (Kind::Login, "title", AtOnce),
        (Kind::Login, field::PURPOSE, AtOnce),
        (Kind::Login, field::WEBSITE, OwnerConfirms),
        (Kind::Login, field::USERNAME, OwnerConfirms),
        (Kind::ApiKey, "title", AtOnce),
        (Kind::ApiKey, field::PURPOSE, AtOnce),
        (Kind::ApiKey, field::EXPIRES, AtOnce),
        (Kind::ApiKey, field::SERVICE, OwnerConfirms),
        (Kind::ApiKey, field::KEY_ID, OwnerConfirms),
        (Kind::ApiKey, field::URL, OwnerConfirms),
        (Kind::ApiKey, field::ENV_VAR, OwnerConfirms),
        (Kind::Text, "title", AtOnce),
    ];
    for (kind, name, rule) in table {
        assert_eq!(agent_edit_rule(*kind, name), *rule, "{kind:?} {name}");
    }
    for kind in Kind::ALL {
        // The table covers every visible field of every Kind.
        for name in kind.visible_fields() {
            assert!(
                table.iter().any(|(k, n, _)| *k == kind && n == name),
                "{kind:?} {name} has no agreed rule"
            );
        }
        // Values are never edited, and nothing outside the Kind is.
        for name in kind.hidden_fields() {
            assert_eq!(agent_edit_rule(kind, name), Never, "{kind:?} {name}");
        }
        for name in [
            "favorite",
            "added_by_agent",
            "deleted_at",
            "kind",
            "id",
            "",
            "Title",
        ] {
            assert_eq!(agent_edit_rule(kind, name), Never, "{kind:?} {name}");
        }
    }
    // A field of another Kind is not this Kind's.
    assert_eq!(agent_edit_rule(Kind::Text, field::PURPOSE), Never);
    assert_eq!(agent_edit_rule(Kind::Login, field::EXPIRES), Never);
    assert_eq!(agent_edit_rule(Kind::PrivateKey, field::WALLET_NAME), Never);
}

#[test]
fn an_edit_needs_the_strictest_rule_of_what_it_changes() {
    use AgentEditRule::{AtOnce, Never, OwnerConfirms, OwnerConfirmsWithPassword};
    assert!(AtOnce < OwnerConfirms);
    assert!(OwnerConfirms < OwnerConfirmsWithPassword);
    assert!(OwnerConfirmsWithPassword < Never);

    assert_eq!(agent_edit_needs(Kind::ApiKey, false, &[]), AtOnce);
    assert_eq!(agent_edit_needs(Kind::ApiKey, true, &[]), AtOnce);
    assert_eq!(
        agent_edit_needs(Kind::ApiKey, true, &[field::PURPOSE, field::EXPIRES]),
        AtOnce
    );
    assert_eq!(
        agent_edit_needs(Kind::ApiKey, true, &[field::PURPOSE, field::URL]),
        OwnerConfirms
    );
    assert_eq!(
        agent_edit_needs(Kind::Login, false, &[field::WEBSITE, field::PASSWORD]),
        Never
    );
    assert_eq!(
        agent_edit_needs(Kind::Login, false, &[field::WEBSITE, "made_up"]),
        Never
    );
    for kind in [Kind::SeedPhrase, Kind::PrivateKey] {
        assert_eq!(
            agent_edit_needs(kind, true, &[]),
            OwnerConfirmsWithPassword,
            "{kind:?}"
        );
        assert_eq!(
            agent_edit_needs(kind, false, &[field::ADDRESS]),
            OwnerConfirmsWithPassword,
            "{kind:?}"
        );
        assert_eq!(
            agent_edit_needs(kind, true, &[field::ADDRESS, kind.hidden_fields()[0]]),
            Never,
            "{kind:?}"
        );
    }
    assert_eq!(
        agent_edit_needs(Kind::SeedPhrase, false, &[field::WORD_COUNT]),
        Never
    );
}
