//! Username Anarchy — high-performance Rust rewrite.
//!
//! Behavioural parity with `urbanadventurer/username-anarchy` (Ruby, v0.6) is
//! the primary goal. See PERFORMANCE.md for the full list of intentionally
//! preserved quirks.

mod engine;
mod fmt;
mod names;
mod util;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::exit;

use engine::{Emitter, GenOpts, GenRet, Scratch};
use fmt::{Ctx, Plugin};
use names::{Name, Person};

const VERSION: &str = "0.6";

const BANNER: &str = r#"
 ___ ____                                                        
|   |    \ ______  ____ _______   ____  _____     __ __    ____  
|   :    //  ___/_/    \\_  __ \ /    \ \__  \   /  :  \ _/    \
'   .   / \___ \ \   o_/ |  | \/|   :  \ /  o \ |  . .  \\   o_/ 
 \_____/ /______) \_____)|__|   |___:  /(______)|__: :  / \_____)
       _____                         \/       .__     \/      
      /     \    ____  _____  _______   ____  |  |__  ___.__.   
     /   o   \  /    \ \__  \ \_  __ \_/ ___\ |  |  \(   :  |   
    /    .    \|   .  \ /  o \ |  | \/\  \___ |   .  \\___  |   
    \____:__  /|___:__/(______)|__|    \_____)|___:__//_____|   
            \/                                                  
"#;

fn usage() -> String {
    let mut s = String::from(BANNER);
    s.push_str("Usage: ./username-anarchy [OPTIONS]... [firstname|first last|first middle last]\n");
    s.push_str(&format!(
        "Author: Andrew Horton (urbanadventurer). Version: {}\n\n",
        VERSION
    ));
    s.push_str(
        r#"Names:
 -i, --input-file FILE     Input list of names. Can be SPACE, CSV or TAB delimited.
                           Defaults to firstname, lastname. Valid column headings are:
                           firstinitial, firstname, lastinitial, lastname,
                           middleinitial, middlename.
 -a, --auto                Automatically generate names from a country/list
 -c, --country COUNTRY     COUNTRY can be one of the following datasets:
                           PublicProfiler:
                           argentina, austria, belgium, canada, china,
                           denmark, france, germany, hungary, india, ireland,
                           italy, luxembourg, netherlands, newzealand, norway,
                           poland, serbia, slovenia, spain, sweden,
                           switzerland, uk, us
                           Other:
                           Facebook - uses the Facebook top 10,000 names
     --given-names FILE    Dictionary of given names
     --family-names FILE   Dictionary of family names
 -s, --substitute STATE    Control name substitutions
                           Valid values are 'on' and 'off'. Default: off
                           Can substitute any part of a name not available
 -m, --max-sub NUM         Limit quantity of substitutions per plugin.
                           Default: -1 (Unlimited)

Username format:
 -l, --list-formats        List format plugins
 -f, --select-format LIST  Select format plugins by name. Comma delimited list
 -r, --recognise USERNAME  Recognise which format is in use for a username.
                           This uses the Facebook dataset. Use verbose mode to
                           show progress.
 -F, --format FORMAT       Define the user format using either format string or
                           ABK format. See README.md for format details.

Output:
 -@, --suffix BOOL         Suffix. e.g. @example.com
                           Default: None
 -C BOOL,                  Case insensitive usernames.
     --case-insensitive    Default: True (All lower case)

Miscellaneous:
 -v, --verbose             Display plugin format comments in output and displays
                           last name searches in plugin format recogniser
 -h, --help
"#,
    );
    s
}

fn fail(msg: &str) -> ! {
    eprintln!("{}", msg);
    exit(1);
}

// ---------------------------------------------------------------------------
// Option identification
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Opt {
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

// ---------------------------------------------------------------------------
// Options state
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Selected {
    All,
    Names,
    Custom,
}

struct Options {
    input_file: Option<String>,
    no_input_names: bool,
    given_names: String,
    family_names: String,
    substitute: bool,
    max_sub: i64,
    selected: Selected,
    selected_names: Vec<String>,
    format_to_recognise: Option<String>,
    suffix: Option<Vec<u8>>,
    case_insensitive: bool,
    verbose: bool,
}

fn root_dir() -> PathBuf {
    if let Ok(r) = std::env::var("USERNAME_ANARCHY_ROOT") {
        return PathBuf::from(r);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            if d.join("names").is_dir() {
                return d.to_path_buf();
            }
        }
    }
    if std::path::Path::new("names").is_dir() {
        if let Ok(cwd) = std::env::current_dir() {
            return cwd;
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
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

fn main() {
    exit(run());
}

fn run() -> i32 {
    let root = root_dir();
    let mut plugins: Vec<Plugin> = fmt::static_plugins();

    let mut opts = Options {
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
    };

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
                    return 1;
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
                            return 1;
                        }
                    }
                }
            } else {
                if inline_val.is_some() {
                    print!("{}", usage());
                    println!();
                    println!("needless argument: --{}", name);
                    return 1;
                }
                String::new()
            };
            if apply_opt(opt, &value, &mut opts, &mut plugins, &root) {
                return 0;
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
                        return 1;
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
                            return 1;
                        }
                    };
                    if apply_opt(opt, &value, &mut opts, &mut plugins, &root) {
                        return 0;
                    }
                    break;
                } else {
                    if apply_opt(opt, "", &mut opts, &mut plugins, &root) {
                        return 0;
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

    // ---- positional names -------------------------------------------------
    let mut people: Vec<Person> = Vec::new();
    match positional.len() {
        1 => people.push(person_of(&positional[0], None, None)),
        2 => people.push(person_of(&positional[0], None, Some(&positional[1]))),
        3 => people.push(person_of(
            &positional[0],
            Some(&positional[1]),
            Some(&positional[2]),
        )),
        _ => {}
    }

    let mut prefix: Vec<u8> = Vec::new();
    if let Some(path) = &opts.input_file {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: No such file or cannot read ({})", path, e);
                return 1;
            }
        };
        match names::load_names_list(&data) {
            Ok(loaded) => {
                prefix.extend_from_slice(&loaded.stdout);
                people.extend(loaded.people);
            }
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        }
    } else if opts.no_input_names {
        // The original pushes an empty `{}` here and (because it only strips it
        // when `no_input_names` is false) leaves it in place in `-a` mode.
        people.push(Default::default());
    } else if people.is_empty() {
        print!("{}", usage());
        return 0;
    }

    // ---- resolve selected plugins ----------------------------------------
    let selected: Vec<Plugin> = match opts.selected {
        Selected::All => plugins.clone(),
        Selected::Names => plugins
            .iter()
            .filter(|p| opts.selected_names.contains(&p.name))
            .cloned()
            .collect(),
        Selected::Custom => plugins
            .iter()
            .filter(|p| "custom".contains(&p.name))
            .cloned()
            .collect(),
    };

    // ---- auto cross product ----------------------------------------------
    if opts.no_input_names && opts.format_to_recognise.is_none() {
        let family = match read_auto_lines(&opts.family_names) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        };
        let given = match read_auto_lines(&opts.given_names) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        };
        people.reserve(family.len().saturating_mul(given.len()));
        for fam in &family {
            for giv in &given {
                let mut p: Person = Default::default();
                p[names::FIRSTNAME] = giv.clone();
                p[names::LASTNAME] = fam.clone();
                people.push(p);
            }
        }
    }

    // ---- generate ---------------------------------------------------------
    let ctx = Ctx::new(opts.given_names.clone(), opts.family_names.clone());
    let gen_opts = GenOpts {
        substitute: opts.substitute,
        max_sub: opts.max_sub,
        verbose: opts.verbose,
        case_insensitive: opts.case_insensitive,
        suffix: opts.suffix.clone(),
    };

    let est = engine::estimate(&people, &selected, &ctx, &gen_opts);
    let threads = std::env::var("USERNAME_ANARCHY_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1)
        });
    // Tunable so the profiling/benchmark harness can force a serial run.
    let min_parallel: u64 = std::env::var("USERNAME_ANARCHY_MIN_PARALLEL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(200_000);
    let parallel = est > min_parallel && threads > 1;

    let generated = if parallel {
        run_parallel(&people, &selected, &ctx, &gen_opts, threads)
    } else {
        run_serial(&people, &selected, &ctx, &gen_opts)
    };
    let generated = match generated {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };

    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    let _ = lock.write_all(&prefix);
    let _ = lock.write_all(&generated);
    let _ = lock.flush();
    0
}

/// Returns `true` if the program should exit successfully (help/list/recognise).
fn apply_opt(
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
            list_plugins(plugins);
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
            opts.suffix = Some(value.to_lowercase().into_bytes())
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

fn person_of(first: &str, middle: Option<&str>, last: Option<&str>) -> Person {
    let mut p: Person = Default::default();
    p[names::FIRSTNAME] = nonempty(first);
    p[names::MIDDLENAME] = middle.and_then(nonempty);
    p[names::LASTNAME] = last.and_then(nonempty);
    p
}

fn nonempty(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() {
        None
    } else {
        Some(s.as_bytes().to_vec())
    }
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

/// `File.open(path).readlines.map { |x| x.downcase.strip! }` used by `--auto`.
fn read_auto_lines(path: &str) -> Result<Vec<Option<Vec<u8>>>, String> {
    let data = std::fs::read(path).map_err(|e| format!("Cannot read file: {} ({})", path, e))?;
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut push_line = |line: &[u8]| {
        let lower = util::downcase(line);
        let stripped = util::strip(&lower);
        if stripped.len() == lower.len() {
            // Ruby `strip!` returned nil -> the value becomes nil.
            out.push(None);
        } else {
            out.push(Some(stripped.to_vec()));
        }
    };
    for i in 0..data.len() {
        if data[i] == b'\n' {
            push_line(&data[start..=i]);
            start = i + 1;
        }
    }
    if start < data.len() {
        push_line(&data[start..]);
    }
    Ok(out)
}

fn run_serial(
    people: &[Person],
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
) -> Result<Vec<u8>, String> {
    let mut em = Emitter::new(opts);
    let mut sc = Scratch::new();
    engine::gen_people(people, plugins, ctx, opts, &mut em, &mut sc)?;
    Ok(em.finish())
}

fn run_parallel(
    people: &[Person],
    plugins: &[Plugin],
    ctx: &Ctx,
    opts: &GenOpts,
    threads: usize,
) -> Result<Vec<u8>, String> {
    let chunk = people.len().div_ceil(threads).max(1);
    let mut err: Option<String> = None;
    let mut sinks: Vec<engine::Sink> = Vec::new();
    std::thread::scope(|s| {
        let handles: Vec<_> = people
            .chunks(chunk)
            .map(|chunk_people| {
                s.spawn(move || {
                    let mut em = Emitter::new(opts);
                    let mut sc = Scratch::new();
                    let r = engine::gen_people(chunk_people, plugins, ctx, opts, &mut em, &mut sc);
                    (r, em.into_sink())
                })
            })
            .collect();
        for h in handles {
            match h.join() {
                Ok((Ok(()), sink)) => sinks.push(sink),
                Ok((Err(e), _)) => {
                    if err.is_none() {
                        err = Some(e);
                    }
                }
                Err(_) => {
                    if err.is_none() {
                        err = Some("worker thread panicked".to_string());
                    }
                }
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    let mut sinks = sinks.into_iter();
    match sinks.next() {
        None => Ok(Vec::new()),
        Some(mut acc) => {
            for other in sinks {
                acc.absorb(other);
            }
            Ok(acc.finish())
        }
    }
}

// ---------------------------------------------------------------------------
// --list-formats
// ---------------------------------------------------------------------------

fn list_plugins(plugins: &[Plugin]) {
    println!("{}\tExample", pad("Plugin name", 20));
    println!("{}", "-".repeat(80));
    let name = Name::init(&person_of("anna", Some("boom"), Some("key")));
    let ctx = Ctx::new(String::new(), String::new());
    let mut sc = Scratch::new();
    for p in plugins {
        let example = match engine::all_outputs(p, &name, &ctx, true, &mut sc) {
            GenRet::Str(s) => String::from_utf8_lossy(&s).into_owned(),
            GenRet::Arr(v) => v
                .iter()
                .take(3)
                .map(|x| String::from_utf8_lossy(x).into_owned())
                .collect::<Vec<_>>()
                .join(","),
            GenRet::Nil => "No example".to_string(),
        };
        println!("{}\t{}", pad(&p.name, 20), example);
    }
}

fn pad(s: &str, width: usize) -> String {
    let len = s.chars().count();
    if len >= width {
        s.to_string()
    } else {
        let mut out = s.to_string();
        out.push_str(&" ".repeat(width - len));
        out
    }
}

// ---------------------------------------------------------------------------
// --recognise
// ---------------------------------------------------------------------------

enum RecogResult {
    Found(String),
    Structure(RNode),
}

enum RNode {
    Nil,
    Arr(Vec<RNode>),
    Plugins,
}

fn render_node(n: &RNode) -> String {
    match n {
        RNode::Nil => "nil".to_string(),
        RNode::Arr(items) => {
            let parts: Vec<String> = items.iter().map(render_node).collect();
            format!("[{}]", parts.join(", "))
        }
        RNode::Plugins => {
            // The original prints Ruby `Plugin` objects whose inspect output
            // embeds a heap address (non-deterministic). We emit a stable
            // structural equivalent.
            let names = ["first"].join("");
            let _ = names;
            "[#<Plugin @plugin_name>...]".to_string()
        }
    }
}

fn first_char(s: &[u8]) -> Vec<u8> {
    let end = util::nth_char_boundary(s, 1);
    s[..end].to_vec()
}

fn meets_letters(letters: &[Vec<u8>], first: &[u8]) -> bool {
    letters.iter().any(|c| c == first)
}

fn recognise(target: &str, opts: &Options, plugins: &[Plugin]) -> RecogResult {
    let mut letters: Vec<Vec<u8>> = target.chars().map(|c| c.to_string().into_bytes()).collect();
    letters.sort();
    letters.dedup();

    let family = match read_stripped_lines(&opts.family_names) {
        Ok(v) => v,
        Err(e) => fail(&e),
    };
    let given = match read_stripped_lines(&opts.given_names) {
        Ok(v) => v,
        Err(e) => fail(&e),
    };

    let ctx = Ctx::new(opts.given_names.clone(), opts.family_names.clone());
    let mut sc = Scratch::new();
    let target_bytes = target.as_bytes();

    let mut outer: Vec<RNode> = Vec::new();
    for fam in &family {
        let fc = first_char(fam);
        if !meets_letters(&letters, &fc) {
            outer.push(RNode::Nil);
            continue;
        }
        if opts.verbose {
            println!("# Searching {}", String::from_utf8_lossy(fam));
        }
        let mut inner: Vec<RNode> = Vec::new();
        for giv in &given {
            let gc = first_char(giv);
            if !meets_letters(&letters, &gc) {
                inner.push(RNode::Nil);
                continue;
            }
            let mut p: Person = Default::default();
            p[names::FIRSTNAME] = Some(giv.clone());
            p[names::LASTNAME] = Some(fam.clone());
            let name = Name::init(&p);
            if opts.verbose {
                println!("{}", String::from_utf8_lossy(&engine::pp_line(&name)));
            }
            let mut found: Option<String> = None;
            for pl in plugins {
                match engine::all_outputs(pl, &name, &ctx, opts.substitute, &mut sc) {
                    GenRet::Str(s) => {
                        if s == target_bytes {
                            found = Some(pl.name.clone());
                        }
                    }
                    GenRet::Arr(v) => {
                        if v.iter().any(|x| x == target_bytes) {
                            found = Some(pl.name.clone());
                        }
                    }
                    GenRet::Nil => {}
                }
                if found.is_some() {
                    break;
                }
            }
            if let Some(n) = found {
                return RecogResult::Found(n);
            }
            inner.push(RNode::Plugins);
        }
        outer.push(RNode::Arr(inner));
    }
    RecogResult::Structure(RNode::Arr(outer))
}

fn read_stripped_lines(path: &str) -> Result<Vec<Vec<u8>>, String> {
    let data = std::fs::read(path).map_err(|e| format!("Cannot read file: {} ({})", path, e))?;
    let lines: Vec<&[u8]> = {
        let mut v = Vec::new();
        let mut start = 0usize;
        for i in 0..data.len() {
            if data[i] == b'\n' {
                v.push(&data[start..=i]);
                start = i + 1;
            }
        }
        if start < data.len() {
            v.push(&data[start..]);
        }
        v
    };
    let n = lines.len();
    let mut out = Vec::with_capacity(n);
    for (i, line) in lines.iter().enumerate() {
        let stripped = util::strip(line);
        if stripped.len() == line.len() && i + 1 == n && !line.ends_with(b"\n") {
            return Err(
                "wordlist last line has no trailing newline; the original raises here".into(),
            );
        }
        out.push(stripped.to_vec());
    }
    Ok(out)
}
