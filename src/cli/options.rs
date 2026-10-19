//! Option state and the side effects of the original option order.
use super::fail;
use super::help::{list_plugins, usage};
use super::parse::Opt;
use super::recognize::{recognise, render_node, RecogResult};
use crate::fmt::{self, Ctx, Plugin};
use crate::util;
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Selected {
    All,
    Names,
    Custom,
}

pub(super) struct Options {
    pub(super) input_file: Option<String>,
    pub(super) no_input_names: bool,
    pub(super) given_names: String,
    pub(super) family_names: String,
    pub(super) substitute: bool,
    pub(super) max_sub: i64,
    pub(super) selected: Selected,
    pub(super) selected_names: Vec<String>,
    pub(super) format_to_recognise: Option<String>,
    pub(super) suffix: Option<Vec<u8>>,
    pub(super) case_insensitive: bool,
    pub(super) verbose: bool,
}

impl Options {
    pub(super) fn new(root: &Path) -> Self {
        Self {
            input_file: None,
            no_input_names: false,
            given_names: root
                .join("names/facebook/firstnames-top10000.txt")
                .to_string_lossy()
                .into_owned(),
            family_names: root
                .join("names/facebook/lastnames-top10000.txt")
                .to_string_lossy()
                .into_owned(),
            substitute: false,
            max_sub: -1,
            selected: Selected::All,
            selected_names: Vec::new(),
            format_to_recognise: None,
            suffix: None,
            case_insensitive: true,
            verbose: false,
        }
    }
}

/// Returns true after handling help, listing or recognition.
pub(super) fn apply_opt(
    opt: Opt,
    value: &str,
    opts: &mut Options,
    plugins: &mut Vec<Plugin>,
    root: &Path,
) -> bool {
    match opt {
        Opt::InputFile => opts.input_file = Some(value.to_string()),
        Opt::Auto => opts.no_input_names = true,
        Opt::Country => {
            let dataset = value.to_lowercase();
            let (g, f) = if dataset == "facebook" {
                (
                    root.join("names/facebook/firstnames-top10000.txt"),
                    root.join("names/facebook/lastnames-top10000.txt"),
                )
            } else {
                (
                    root.join(format!("names/publicprofiler/{}-forenames.txt", dataset)),
                    root.join(format!("names/publicprofiler/{}-surnames.txt", dataset)),
                )
            };
            if !Ctx::readable(&g.to_string_lossy()) {
                fail(&format!("Cannot read file: {}", g.display()));
            }
            if !Ctx::readable(&f.to_string_lossy()) {
                fail(&format!("Cannot read file: {}", f.display()));
            }
            opts.given_names = g.to_string_lossy().into_owned();
            opts.family_names = f.to_string_lossy().into_owned();
        }
        Opt::GivenNames => {
            if Ctx::readable(&opts.given_names) {
                opts.given_names = value.to_string();
            } else {
                fail(&format!("Cannot read file: {}", value));
            }
        }
        Opt::FamilyNames => {
            if Ctx::readable(&opts.family_names) {
                opts.family_names = value.to_string();
            } else {
                fail(&format!("Cannot read file: {}", value));
            }
        }
        Opt::Substitute => opts.substitute = parse_explicit_sub_state(value),
        Opt::MaxSub => {
            let n = match parse_integer(value) {
                Some(n) => n,
                None => fail(&format!("invalid argument: -m {}", value)),
            };
            if n > 0 {
                opts.max_sub = n - 1;
            } else {
                fail("Fatal Error. maximum substitutions must be a positive integer");
            }
        }
        Opt::ListFormats => {
            opts.substitute = true;
            list_plugins(plugins, opts.verbose);
            return true;
        }
        Opt::SelectFormat => {
            // OptionParser's `Array` coercion: `val.split(/,/)` with Ruby's
            // trailing-empty removal, and empty fields mapped to nil.
            let list = ruby_comma_split(value);
            for x in &list {
                let known = matches!(x, Some(n) if plugins.iter().any(|p| &p.name == n));
                if !known {
                    fail(&format!(
                        "Fatal Error. Format plugin not found: {}.",
                        x.as_deref().unwrap_or("")
                    ));
                }
            }
            opts.selected = Selected::Names;
            opts.selected_names = list.into_iter().flatten().collect();
        }
        Opt::Recognise => {
            let target = value.to_string();
            opts.format_to_recognise = Some(target.clone());
            opts.no_input_names = true;
            println!("Recognising {}. This can take a while.", target);
            let result = recognise(&target, opts, plugins);
            match result {
                RecogResult::Found(name) => println!(
                    "Username format {} recognised. Plugin name: {}",
                    target, name
                ),
                RecogResult::Structure(node) => println!(
                    "Username format {} recognised. Plugin name: {}",
                    target,
                    render_node(&node)
                ),
            }
            return true;
        }
        Opt::Format => {
            let fmt = value.replace('"', "");
            let p = fmt::custom_plugin(&fmt);
            plugins.push(p);
            opts.selected = Selected::Custom;
        }
        Opt::Suffix => {
            if value.is_empty() {
                fail("invalid argument: --suffix (OptionParser::InvalidArgument)");
            }
            opts.suffix = Some(util::downcase(value.as_bytes()))
        }
        Opt::CaseInsensitive => {
            if value.is_empty() {
                fail("invalid argument: --case-insensitive (OptionParser::InvalidArgument)");
            }
            let v = value.to_lowercase();
            match v.as_str() {
                "true" => opts.case_insensitive = true,
                "false" => opts.case_insensitive = false,
                _ => fail("Fatal Error. Invalid option. Must be true or false"),
            }
        }
        Opt::Verbose => opts.verbose = true,
        Opt::Help => {
            print!("{}", usage());
            return true;
        }
    }
    false
}

fn parse_explicit_sub_state(v: &str) -> bool {
    match v {
        "on" => true,
        "off" => false,
        _ => fail("Fatal Error. invalid argument for --substitute"),
    }
}

/// Ruby `Integer()`-ish parsing used by OptionParser's Integer coercion.
fn parse_integer(v: &str) -> Option<i64> {
    let t = v.trim();
    if t.is_empty() {
        return None;
    }
    let (sign, rest) = if let Some(r) = t.strip_prefix('-') {
        (-1i64, r)
    } else if let Some(r) = t.strip_prefix('+') {
        (1i64, r)
    } else {
        (1i64, t)
    };
    let (radix, digits) =
        if let Some(r) = rest.strip_prefix("0x").or_else(|| rest.strip_prefix("0X")) {
            (16u32, r)
        } else if let Some(r) = rest.strip_prefix("0b").or_else(|| rest.strip_prefix("0B")) {
            (2u32, r)
        } else if let Some(r) = rest.strip_prefix("0o").or_else(|| rest.strip_prefix("0O")) {
            (8u32, r)
        } else if rest.len() > 1 && rest.starts_with('0') {
            // Ruby's `Integer()` treats a bare leading zero as octal.
            (8u32, &rest[1..])
        } else {
            (10u32, rest)
        };
    let cleaned: String = digits.chars().filter(|&c| c != '_').collect();
    if cleaned.is_empty() {
        return None;
    }
    i64::from_str_radix(&cleaned, radix).ok().map(|n| n * sign)
}

/// Ruby `String#split(',')` followed by OptionParser's `Array` coercion, which
/// maps empty fields to nil: `""` and `","` both yield an empty list, while
/// `",first"` yields `[nil, "first"]`.
fn ruby_comma_split(s: &str) -> Vec<Option<String>> {
    let mut parts: Vec<&str> = s.split(',').collect();
    while parts.last().map(|p| p.is_empty()).unwrap_or(false) {
        parts.pop();
    }
    parts
        .iter()
        .map(|p| {
            if p.is_empty() {
                None
            } else {
                Some(p.to_string())
            }
        })
        .collect()
}
