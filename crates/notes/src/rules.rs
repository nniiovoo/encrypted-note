//! Rules for the values of the visible fields that document schema 2 added (FORMAT.md
//! section 8).
//!
//! Each rule is checked twice: when the owner supplies a value (create and update answer
//! `NotesError::InvalidValue`, naming the field) and when a body is parsed (`Corrupt`). Fields
//! that schema 1 already had carry no rule, because Vaults written before these rules existed
//! must keep opening.

use crate::{Kind, field};

/// The longest `purpose`, in characters.
pub const MAX_PURPOSE_CHARS: usize = 200;
/// The longest `env_var`, in characters.
pub const MAX_ENV_VAR_CHARS: usize = 128;

/// True if `value` (not empty) is allowed in the visible field `name` of `kind`.
///
/// * `expires`: a date written `YYYY-MM-DD` that exists.
/// * `env_var`: `[A-Za-z_][A-Za-z0-9_]*`, at most [`MAX_ENV_VAR_CHARS`] characters.
/// * `purpose`: one line of at most [`MAX_PURPOSE_CHARS`] characters.
/// * `key_id`, `url`, `derivation_path` and the `address` of a Seed Phrase: one line.
///
/// "One line" means no control characters, which rules out line breaks and tabs.
pub(crate) fn allows(kind: Kind, name: &str, value: &str) -> bool {
    match (kind, name) {
        (Kind::ApiKey, field::EXPIRES) => is_calendar_date(value),
        (Kind::ApiKey, field::ENV_VAR) => is_variable_name(value),
        (Kind::ApiKey | Kind::Login, field::PURPOSE) => {
            is_one_line(value) && value.chars().count() <= MAX_PURPOSE_CHARS
        }
        (Kind::ApiKey, field::KEY_ID | field::URL)
        | (Kind::SeedPhrase, field::ADDRESS | field::DERIVATION_PATH) => is_one_line(value),
        _ => true,
    }
}

fn is_one_line(value: &str) -> bool {
    !value.chars().any(char::is_control)
}

/// `[A-Za-z_][A-Za-z0-9_]*`: the names every shell and most languages accept for a variable.
fn is_variable_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && value.len() <= MAX_ENV_VAR_CHARS
}

/// Exactly `YYYY-MM-DD` in ASCII digits, naming a day that exists in the Gregorian calendar
/// (so 29 February only in a leap year).
fn is_calendar_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let number = |digits: &[u8]| {
        digits.iter().try_fold(0u32, |n, d| {
            d.is_ascii_digit().then(|| n * 10 + u32::from(d - b'0'))
        })
    };
    let (Some(year), Some(month), Some(day)) = (
        number(&bytes[0..4]),
        number(&bytes[5..7]),
        number(&bytes[8..10]),
    ) else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days_in_month).contains(&day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_must_be_written_yyyy_mm_dd_and_exist() {
        for good in [
            "2027-01-31",
            "2028-02-29",
            "2000-02-29",
            "2027-12-01",
            "9999-12-31",
        ] {
            assert!(is_calendar_date(good), "{good}");
        }
        for bad in [
            "",
            "2027-1-31",
            "27-01-31",
            "2027/01/31",
            "2027-01-31 ",
            "2027-01-32",
            "2027-00-10",
            "2027-13-01",
            "2027-04-31",
            "2027-02-29",
            "1900-02-29",
            "2027-01-00",
            "20270131",
            "2027-01-3x",
            "+027-01-31",
            "2027-01-31T00:00",
            "\u{0662}027-01-31",
        ] {
            assert!(!is_calendar_date(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_variable_name_is_letters_digits_and_underscores_not_starting_with_a_digit() {
        for good in ["FAKE_API_KEY", "_x", "a", "fake_key_2"] {
            assert!(is_variable_name(good), "{good}");
        }
        for bad in [
            "",
            "2FAKE",
            "FAKE-KEY",
            "FAKE KEY",
            "FAKE=1",
            "$FAKE",
            "FAK\u{c9}",
        ] {
            assert!(!is_variable_name(bad), "{bad:?}");
        }
        assert!(is_variable_name(&"A".repeat(MAX_ENV_VAR_CHARS)));
        assert!(!is_variable_name(&"A".repeat(MAX_ENV_VAR_CHARS + 1)));
    }

    #[test]
    fn one_line_means_no_control_characters() {
        assert!(is_one_line(
            "fake value with spaces, caf\u{e9} and \u{65e5}\u{672c}"
        ));
        for bad in [
            "two\nlines",
            "carriage\rreturn",
            "a\ttab",
            "escape\u{1b}[2J",
            "c1\u{85}",
        ] {
            assert!(!is_one_line(bad), "{bad:?}");
        }
    }

    #[test]
    fn fields_from_schema_1_have_no_rule() {
        let anything = "fake\nvalue\twith control characters";
        for (kind, name) in [
            (Kind::PrivateKey, field::ADDRESS),
            (Kind::PrivateKey, field::CHAIN),
            (Kind::Login, field::WEBSITE),
            (Kind::Login, field::USERNAME),
            (Kind::ApiKey, field::SERVICE),
            (Kind::SeedPhrase, field::WALLET_NAME),
        ] {
            assert!(allows(kind, name, anything), "{kind:?} {name}");
        }
        // The same field name has a rule on a Seed Phrase, where schema 2 added it.
        assert!(!allows(Kind::SeedPhrase, field::ADDRESS, anything));
    }
}
