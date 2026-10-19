use super::*;

fn person<'a>(cols: [&'a str; 6]) -> Person<'a> {
    cols.map(|s| {
        if s.is_empty() {
            None
        } else {
            Some(s.as_bytes())
        }
    })
}

/// `NameRef::from_person` must be indistinguishable from `Name::init`,
/// including the original's `middleinitial`/`lastinitial` copy/paste bug.
#[test]
fn from_person_matches_init() {
    let cases = [
        ["anna", "key", "", "", "", ""],
        ["anna", "key", "", "", "boom", ""],
        ["Anna", "Key", "A", "K", "Boom", "M"],
        ["anna", "key", "", "K", "", ""],
        ["anna", "key", "", "", "", "M"],
        ["", "", "", "", "", ""],
        ["é", "key", "", "", "", ""],
        ["anna", "key", "É", "", "", ""],
    ];
    for c in cases {
        let p = person(c);
        let owned = Name::init(&p);
        let borrowed = NameRef::from_person(&p);
        assert_eq!(owned.firstname.as_deref(), borrowed.firstname, "{c:?}");
        assert_eq!(owned.lastname.as_deref(), borrowed.lastname, "{c:?}");
        assert_eq!(owned.middlename.as_deref(), borrowed.middlename, "{c:?}");
        assert_eq!(
            owned.firstinitial.as_deref(),
            borrowed.firstinitial,
            "{c:?}"
        );
        assert_eq!(owned.lastinitial.as_deref(), borrowed.lastinitial, "{c:?}");
        assert_eq!(
            owned.middleinitial.as_deref(),
            borrowed.middleinitial,
            "{c:?}"
        );
    }
}
