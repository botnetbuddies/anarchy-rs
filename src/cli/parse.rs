//! Short-option clusters, long-option abbreviations and argument order.
use super::help::usage;
use super::input::root_dir;
use super::options::{apply_opt, Options};
use crate::fmt::{self, Plugin};
use std::ops::ControlFlow;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Opt {
    InputFile,
    Auto,
    Country,
    GivenNames,
    FamilyNames,
    Substitute,
    MaxSub,
    ListFormats,
    SelectFormat,
    Recognise,
    Format,
    Suffix,
    CaseInsensitive,
    Verbose,
    Help,
}

impl Opt {
    fn takes_arg(self) -> bool {
        !matches!(
            self,
            Opt::Auto | Opt::ListFormats | Opt::Verbose | Opt::Help
        )
    }
}

const LONG_OPTS: [(&str, Opt); 15] = [
    ("input-file", Opt::InputFile),
    ("auto", Opt::Auto),
    ("country", Opt::Country),
    ("given-names", Opt::GivenNames),
    ("family-names", Opt::FamilyNames),
    ("substitute", Opt::Substitute),
    ("max-sub", Opt::MaxSub),
    ("list-formats", Opt::ListFormats),
    ("select-format", Opt::SelectFormat),
    ("recognise", Opt::Recognise),
    ("format", Opt::Format),
    ("suffix", Opt::Suffix),
    ("case-insensitive", Opt::CaseInsensitive),
    ("verbose", Opt::Verbose),
    ("help", Opt::Help),
];

fn short_opt(c: char) -> Option<Opt> {
    Some(match c {
        'i' => Opt::InputFile,
        'a' => Opt::Auto,
        'c' => Opt::Country,
        's' => Opt::Substitute,
        'm' => Opt::MaxSub,
        'l' => Opt::ListFormats,
        'f' => Opt::SelectFormat,
        'r' => Opt::Recognise,
        'F' => Opt::Format,
        '@' => Opt::Suffix,
        'C' => Opt::CaseInsensitive,
        'v' => Opt::Verbose,
        'h' => Opt::Help,
        _ => return None,
    })
}

/// Resolve a long option name, reproducing OptionParser's unambiguous-prefix
/// abbreviation.
fn resolve_long(prefix: &str) -> Result<Opt, String> {
    if prefix.is_empty() {
        return Err("invalid option".to_string());
    }
    let matches: Vec<(&str, Opt)> = LONG_OPTS
        .iter()
        .filter(|(n, _)| n.starts_with(prefix))
        .cloned()
        .collect();
    match matches.len() {
        1 => Ok(matches[0].1),
        0 => Err(format!("invalid option: --{}", prefix)),
        _ => Err(format!("ambiguous option: --{}", prefix)),
    }
}

pub(super) struct Parsed {
    pub options: Options,
    pub plugins: Vec<Plugin>,
    pub positional: Vec<String>,
}

pub(super) fn parse_args() -> ControlFlow<i32, Parsed> {
    let root = root_dir();
    let mut plugins = fmt::static_plugins();
    let mut opts = Options::new(&root);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional: Vec<String> = Vec::new();

    let mut i = 0usize;
    let mut no_more_opts = false;
    while i < args.len() {
        let a = args[i].clone();
        if no_more_opts {
            positional.push(a);
            i += 1;
            continue;
        }
        if a == "--" {
            no_more_opts = true;
            i += 1;
            continue;
        }
        if let Some(rest) = a.strip_prefix("--") {
            let (name, inline_val) = match rest.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (rest, None),
            };
            let opt = match resolve_long(name) {
                Ok(o) => o,
                Err(e) => {
                    print!("{}", usage());
                    println!();
                    println!("{}", e);
                    return ControlFlow::Break(1);
                }
            };
            let value = if opt.takes_arg() {
                match inline_val {
                    Some(v) => v,
                    None => {
                        if i + 1 < args.len() {
                            i += 1;
                            args[i].clone()
                        } else {
                            print!("{}", usage());
                            println!();
                            println!("missing argument: --{}", name);
                            return ControlFlow::Break(1);
                        }
                    }
                }
            } else {
                if inline_val.is_some() {
                    print!("{}", usage());
                    println!();
                    println!("needless argument: --{}", name);
                    return ControlFlow::Break(1);
                }
                String::new()
            };
            if apply_opt(opt, &value, &mut opts, &mut plugins, &root) {
                return ControlFlow::Break(0);
            }
            i += 1;
            continue;
        }
        if a.len() > 1 && a.starts_with('-') {
            let chars: Vec<char> = a.chars().skip(1).collect();
            let mut j = 0usize;
            let mut consumed_next = false;
            while j < chars.len() {
                let c = chars[j];
                let opt = match short_opt(c) {
                    Some(o) => o,
                    None => {
                        print!("{}", usage());
                        println!();
                        println!("invalid option: -{}", c);
                        return ControlFlow::Break(1);
                    }
                };
                if opt.takes_arg() {
                    let rest: String = chars[j + 1..].iter().collect();
                    let value = if !rest.is_empty() {
                        rest
                    } else {
                        if i + 1 < args.len() {
                            consumed_next = true;
                            args[i + 1].clone()
                        } else {
                            print!("{}", usage());
                            println!();
                            println!("missing argument: -{}", c);
                            return ControlFlow::Break(1);
                        }
                    };
                    if apply_opt(opt, &value, &mut opts, &mut plugins, &root) {
                        return ControlFlow::Break(0);
                    }
                    break;
                } else {
                    if apply_opt(opt, "", &mut opts, &mut plugins, &root) {
                        return ControlFlow::Break(0);
                    }
                    j += 1;
                }
            }
            i += if consumed_next { 2 } else { 1 };
            continue;
        }
        positional.push(a);
        i += 1;
    }

    ControlFlow::Continue(Parsed {
        options: opts,
        plugins,
        positional,
    })
}
