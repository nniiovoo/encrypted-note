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
        &[field::WALLET_NAME, field::WORD_COUNT]
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
        &[field::WEBSITE, field::USERNAME]
    );
    assert_eq!(Kind::Login.hidden_fields(), &[field::PASSWORD]);
    assert_eq!(Kind::ApiKey.visible_fields(), &[field::SERVICE]);
    assert_eq!(Kind::ApiKey.hidden_fields(), &[field::KEY]);
    assert!(Kind::Text.visible_fields().is_empty());
    assert_eq!(Kind::Text.hidden_fields(), &[field::BODY]);
}
