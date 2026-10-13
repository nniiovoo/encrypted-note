use notes::{Kind, field};

#[test]
fn wallet_kinds_are_seed_phrase_and_private_key() {
    let wallets: Vec<Kind> = Kind::ALL.into_iter().filter(|k| k.is_wallet()).collect();
    assert_eq!(wallets, vec![Kind::SeedPhrase, Kind::PrivateKey]);
}

#[test]
fn field_maps_per_kind() {
    assert_eq!(
        Kind::SeedPhrase.visible_fields(),
        &[
            field::WALLET_NAME,
            field::WORD_COUNT,
            field::ADDRESS,
            field::DERIVATION_PATH
        ]
    );
    assert_eq!(
        Kind::SeedPhrase.hidden_fields(),
        &[field::WORDS, field::PASSPHRASE]
    );
    assert_eq!(
        Kind::PrivateKey.visible_fields(),
        &[field::CHAIN, field::ADDRESS]
    );
    assert_eq!(Kind::PrivateKey.hidden_fields(), &[field::KEY]);
    assert_eq!(
        Kind::Login.visible_fields(),
        &[field::WEBSITE, field::USERNAME, field::PURPOSE]
    );
    assert_eq!(Kind::Login.hidden_fields(), &[field::PASSWORD]);
    assert_eq!(
        Kind::ApiKey.visible_fields(),
        &[
            field::SERVICE,
            field::KEY_ID,
            field::PURPOSE,
            field::URL,
            field::EXPIRES,
            field::ENV_VAR
        ]
    );
    assert_eq!(Kind::ApiKey.hidden_fields(), &[field::KEY, field::SECRET]);
    assert!(Kind::Text.visible_fields().is_empty());
    assert_eq!(Kind::Text.hidden_fields(), &[field::BODY]);
}

#[test]
fn the_fields_schema_2_added_are_these_and_nothing_else() {
    assert_eq!(
        Kind::SeedPhrase.schema_2_fields(),
        &[field::ADDRESS, field::DERIVATION_PATH]
    );
    assert_eq!(Kind::Login.schema_2_fields(), &[field::PURPOSE]);
    assert_eq!(
        Kind::ApiKey.schema_2_fields(),
        &[
            field::KEY_ID,
            field::PURPOSE,
            field::URL,
            field::EXPIRES,
            field::ENV_VAR,
            field::SECRET
        ]
    );
    // A Private Key had its address from the start, and Text gained nothing.
    assert!(Kind::PrivateKey.schema_2_fields().is_empty());
    assert!(Kind::Text.schema_2_fields().is_empty());
}

#[test]
fn every_schema_2_field_belongs_to_its_kind_and_none_is_a_wallet_field() {
    for kind in Kind::ALL {
        for name in kind.schema_2_fields() {
            let visible = kind.visible_fields().contains(name);
            let hidden = kind.hidden_fields().contains(name);
            assert!(visible != hidden, "{kind:?} {name}");
            // No new sealed field: wallet field encryption and Key Rotation are untouched.
            assert!(!(kind.is_wallet() && hidden), "{kind:?} {name}");
        }
    }
}
