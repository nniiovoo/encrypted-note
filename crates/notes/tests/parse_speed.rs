//! Parsing a Vault document must cost time in proportion to its size.
//!
//! `Document::from_json` runs on every unlock and on the read-back check of every save. The
//! vendored serde_json wipes its temporary string buffer while parsing (`vendor/serde_json`,
//! see VENDORED.md there). An earlier version of that patch wiped the whole buffer, which is as
//! long as the document, before every JSON string, so a few thousand Notes took minutes. This
//! test fails if that comes back. Every value here is an obvious fake.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use notes::{Document, Kind, NewNote, field};
use zeroize::Zeroizing;

const T0: i64 = 1_700_000_000;

/// Enough Notes that a wipe of the whole document per JSON string takes minutes in a debug
/// build, while a parse that only wipes what it used takes well under a second.
const NOTES: usize = 3_000;

/// Generous for a slow machine running a debug build under load. Measured on one Mac, debug
/// build: 0.08 s with the parser as it is now, 662 s with the quadratic one.
const LIMIT: Duration = Duration::from_secs(15);

fn one(key: &str, value: String) -> BTreeMap<String, Zeroizing<String>> {
    BTreeMap::from([(key.to_string(), Zeroizing::new(value))])
}

/// A Login Note with plain values: its strings need no unescaping.
fn fake_login(i: usize) -> NewNote {
    NewNote {
        kind: Kind::Login,
        title: format!("Fake login {i}"),
        visible: BTreeMap::from([
            (field::WEBSITE.to_string(), format!("site-{i}.example.test")),
            (field::USERNAME.to_string(), format!("fake-user-{i}")),
        ]),
        hidden: one(field::PASSWORD, format!("fake-password-{i}-not-a-secret")),
    }
}

/// A Text Note whose body has line breaks, quotes and a backslash, so the parser has to
/// unescape it through its temporary buffer.
fn fake_text_body(i: usize) -> String {
    format!("fake line one of note {i}\nfake \"quoted\" line\nfake\\path\tend {i}\n").repeat(3)
}

fn fake_text(i: usize) -> NewNote {
    NewNote {
        kind: Kind::Text,
        title: format!("Fake \"text\" {i}"),
        visible: BTreeMap::new(),
        hidden: one(field::BODY, fake_text_body(i)),
    }
}

#[test]
fn parsing_a_few_thousand_notes_is_not_quadratic() {
    let mut doc = Document::new(T0);
    let mut ids = Vec::with_capacity(NOTES);
    for i in 0..NOTES {
        let note = if i % 2 == 0 {
            fake_login(i)
        } else {
            fake_text(i)
        };
        ids.push(doc.create(note, None, T0).unwrap());
    }
    let json = doc.to_json();

    let started = Instant::now();
    let parsed = Document::from_json(&json).unwrap();
    let took = started.elapsed();
    eprintln!(
        "parsed {NOTES} fake Notes ({} bytes) in {took:?}",
        json.len()
    );

    // The parse still has to be right: check escaped and plain values from the start, the
    // middle and the end, where a wrongly wiped buffer would show.
    assert_eq!(parsed.count(), NOTES);
    for i in [0, 1, NOTES / 2, NOTES / 2 + 1, NOTES - 2, NOTES - 1] {
        let (name, expected) = if i % 2 == 0 {
            (field::PASSWORD, format!("fake-password-{i}-not-a-secret"))
        } else {
            (field::BODY, fake_text_body(i))
        };
        assert_eq!(parsed.reveal(&ids[i], name).unwrap().as_str(), expected);
        assert_eq!(parsed.get(&ids[i]), doc.get(&ids[i]));
    }

    assert!(
        took < LIMIT,
        "parsing {NOTES} Notes ({} bytes) took {took:?}, limit {LIMIT:?}: \
         is the parser wiping its whole buffer for every string again?",
        json.len()
    );
}
