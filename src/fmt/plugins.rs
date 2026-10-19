//! Built-in format registration and custom-format setup.
use super::template::{abk_to_format, compile_ascii_parts, compute_required_mask, parse_template};
use super::{Plugin, PluginKind};

fn format_plugin(name: &str, raw: &str, abk: bool, strict: bool) -> Plugin {
    let expanded = if abk {
        abk_to_format(raw)
    } else {
        raw.to_string()
    };
    let (segs, lits) = parse_template(&expanded);
    let ascii_parts = if strict {
        None
    } else {
        compile_ascii_parts(&segs, &lits)
    };
    let sequential_source =
        (strict && expanded.contains("%%")).then(|| expanded.as_bytes().to_vec());
    let kind = PluginKind::Format { segs, lits };
    let required_mask = compute_required_mask(&kind);
    Plugin {
        name: name.to_string(),
        kind,
        strict,
        required_mask,
        ascii_parts,
        ascii_duplicate: false,
        sequential_source,
    }
}

/// Run after format selection: a subset may omit the earlier equivalent format.
pub fn mark_ascii_duplicates(plugins: &mut [Plugin]) {
    for i in 0..plugins.len() {
        plugins[i].ascii_duplicate = plugins[i].ascii_parts.is_some()
            && plugins[..i]
                .iter()
                .any(|p| p.ascii_parts == plugins[i].ascii_parts);
    }
}

/// The 24 static plugins, in registration order (`format-plugins.rb`).
pub fn static_plugins() -> Vec<Plugin> {
    vec![
        format_plugin("first", "%f", false, false),
        format_plugin("firstlast", "annakey", true, false),
        format_plugin("first.last", "anna.key", true, false),
        Plugin {
            name: "firstlast[8]".to_string(),
            kind: PluginKind::TruncFirstLast8,
            strict: false,
            required_mask: (1 << crate::names::FIRSTNAME) | (1 << crate::names::LASTNAME),
            ascii_parts: None,
            ascii_duplicate: false,
            sequential_source: None,
        },
        Plugin {
            name: "first[4]last[4]".to_string(),
            kind: PluginKind::TruncFirst4Last4,
            strict: false,
            required_mask: (1 << crate::names::FIRSTNAME) | (1 << crate::names::LASTNAME),
            ascii_parts: None,
            ascii_duplicate: false,
            sequential_source: None,
        },
        format_plugin("firstl", "annak", true, false),
        format_plugin("f.last", "a.key", true, false),
        format_plugin("flast", "akey", true, false),
        format_plugin("lfirst", "kanna", true, false),
        format_plugin("l.first", "k.anna", true, false),
        format_plugin("lastf", "keya", true, false),
        format_plugin("last", "key", true, false),
        format_plugin("last.f", "key.a", true, false),
        format_plugin("last.first", "key.anna", true, false),
        format_plugin("FLast", "AKey", true, false),
        format_plugin("first1", "anna%D", true, false),
        format_plugin("fl", "ak", true, false),
        format_plugin("fmlast", "abkey", true, false),
        format_plugin("firstmiddlelast", "annaboomkey", true, false),
        format_plugin("fml", "abk", true, false),
        format_plugin("FL", "AK", true, false),
        format_plugin("FirstLast", "%F%L", true, false),
        format_plugin("First.Last", "%F.%L", true, false),
        format_plugin("Last", "Key", true, false),
    ]
}

/// Custom `-F` plugin: always routed through the ABK translator (`format_anna`).
/// Uses the faithful round-based expansion because arbitrary formats can
/// reassemble tokens from expanded fragments.
pub fn custom_plugin(fmt: &str) -> Plugin {
    format_plugin("custom", fmt, true, true)
}
