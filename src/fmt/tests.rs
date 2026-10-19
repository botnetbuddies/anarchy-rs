use super::*;
use crate::names::{NameRef, Person};

fn person<'a>(cols: [&'a str; 6]) -> Person<'a> {
    cols.map(|s| {
        if s.is_empty() {
            None
        } else {
            Some(s.as_bytes())
        }
    })
}

#[test]
fn field_props_classifies() {
    let props = |c: [&str; 6]| field_props(&NameRef::from_person(&person(c)));
    assert_eq!(props(["anna", "key", "", "", "", ""]), (true, true));
    assert_eq!(props(["a", "k", "b", "", "", ""]), (true, true));
    // Uppercase forces the folding path.
    assert_eq!(props(["Anna", "key", "", "", "", ""]), (false, true));
    // Non-ASCII forces the folding path.
    assert_eq!(props(["é", "key", "", "", "", ""]), (false, true));
    // A percent sign may be an expansion token inside a supplied name.
    assert_eq!(props(["ann%d", "key", "", "", "", ""]), (false, true));
    // A space or apostrophe is `plain` but not `clean`.
    assert_eq!(props(["mary jane", "key", "", "", "", ""]), (true, false));
    assert_eq!(props(["o'brien", "key", "", "", "", ""]), (true, false));
}

/// The raw resolver is only used for `plain` names, where it must produce
/// exactly what the general `downcase(upcase_it(x))` resolver produces.
#[test]
fn raw_lower_matches_folding_for_plain_names() {
    let cases = [
        ["anna", "key", "", "", "", ""],
        ["a", "k", "b", "", "", ""],
        ["mary", "jane", "o'brien", "", "", ""],
        ["anna", "key", "", "K", "", ""],
    ];
    let plugins = static_plugins();
    for c in cases {
        let p = person(c);
        let name = NameRef::from_person(&p);
        assert!(field_props(&name).0, "{c:?} should be plain");
        for p in &plugins {
            if let PluginKind::Format { segs, lits } = &p.kind {
                let mut folded = Vec::new();
                let mut raw = Vec::new();
                resolve_string_lower(segs, lits, &name, &mut folded);
                resolve_string_lower_raw(segs, lits, &name, &mut raw);
                assert_eq!(folded, raw, "{} {c:?}", p.name);
            }
        }
    }
}
