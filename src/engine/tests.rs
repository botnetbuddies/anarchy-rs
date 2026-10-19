use super::*;
use crate::fmt::{self, Ctx};
use crate::names::person_of;

fn opts() -> GenOpts {
    GenOpts {
        substitute: false,
        max_sub: -1,
        verbose: true,
        case_insensitive: false,
        suffix: Some(b"@example.com".to_vec()),
    }
}

#[test]
fn ascii_templates_match_general_engine_including_fallbacks() {
    let long = "x".repeat(2000);
    let people = [
        person_of("anna", Some("boom"), Some("key")),
        person_of("a", Some("b"), Some("k")),
        person_of("anna", None, Some("key")),
        person_of("anna", None, None),
        person_of("", None, Some("key")),
        person_of("mary jane", None, Some("o'brien")),
        person_of("Élodie", None, Some("Dubois")),
        person_of("ann%givenname", None, Some("key")),
        person_of("anna", Some("b%d"), Some("key")),
        person_of(&long, None, Some("key")),
    ];
    let work = Work::Fixed(&people);
    let mut plugins = fmt::static_plugins();
    fmt::mark_ascii_duplicates(&mut plugins);
    let mut general = plugins.clone();
    for plugin in &mut general {
        plugin.ascii_parts = None;
    }
    let ctx = Ctx::new(String::new(), String::new());
    for verbose in [false, true] {
        for suffix in [None, Some(b"@example.com".to_vec())] {
            let mut opts = opts();
            opts.case_insensitive = true;
            opts.verbose = verbose;
            opts.suffix = suffix;
            assert_eq!(
                run_serial(work, &plugins, &ctx, &opts, 16_384).unwrap(),
                run_serial(work, &general, &ctx, &opts, 16_384).unwrap()
            );
        }
    }
}

#[test]
fn parallel_fixed_matches_serial_across_chunk_boundaries() {
    let people: Vec<_> = (0..1600)
        .map(|i| match i % 3 {
            0 => person_of("Anna", Some("Boom"), Some("Key")),
            1 => person_of("Élodie", None, Some("Dubois")),
            _ => person_of("anna", None, None),
        })
        .collect();
    let work = Work::Fixed(&people);
    let plugins = fmt::static_plugins();
    let ctx = Ctx::new(String::new(), String::new());
    let opts = opts();
    assert_eq!(
        run_parallel(work, &plugins, &ctx, &opts, 4, 40_000).unwrap(),
        run_serial(work, &plugins, &ctx, &opts, 40_000).unwrap()
    );
}

#[test]
fn parallel_auto_matches_serial_with_head_and_duplicate_names() {
    let given = vec![Some(b"anna".to_vec()), Some(b"bob".to_vec()), None];
    let family: Vec<_> = (0..400)
        .map(|i| Some(format!("family{}", i % 7).into_bytes()))
        .collect();
    let head = [person_of("zed", None, Some("key"))];
    let work = Work::Auto {
        head: &head,
        names: AutoNames {
            given: &given,
            family: &family,
        },
    };
    let plugins = fmt::static_plugins();
    let ctx = Ctx::new(String::new(), String::new());
    let opts = opts();
    assert_eq!(
        run_parallel(work, &plugins, &ctx, &opts, 4, 30_000).unwrap(),
        run_serial(work, &plugins, &ctx, &opts, 30_000).unwrap()
    );
}

#[test]
fn worker_error_disconnects_producers_before_join() {
    let mut people = vec![person_of("anna", None, Some("key")); 32_768];
    people[0] = person_of("anna", None, None);
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/cli/fixtures/given_nonl.txt"
    );
    let ctx = Ctx::new(fixture.into(), fixture.into());
    let plugins: Vec<_> = fmt::static_plugins()
        .into_iter()
        .filter(|p| p.name == "firstlast")
        .collect();
    let mut opts = opts();
    opts.substitute = true;
    opts.verbose = false;
    let result = run_parallel(Work::Fixed(&people), &plugins, &ctx, &opts, 4, 32_768);
    assert!(result.unwrap_err().contains("no trailing newline"));
}
