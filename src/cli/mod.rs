//! CLI orchestration: parse, load, select formats, generate and write.
mod help;
mod input;
mod options;
mod parse;
mod recognize;

use crate::engine::{self, run_parallel, run_serial, AutoNames, GenOpts, Work};
use crate::fmt::{self, Ctx, Plugin};
use crate::names::{self, person_of, Person};
use help::usage;
use input::read_auto_lines;
use options::Selected;
use std::io::Write;
use std::ops::ControlFlow;

pub(crate) fn run() -> i32 {
    let parse::Parsed {
        options: opts,
        plugins,
        positional,
    } = match parse::parse_args() {
        ControlFlow::Continue(parsed) => parsed,
        ControlFlow::Break(code) => return code,
    };
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

    let mut data: Vec<u8> = match &opts.input_file {
        Some(path) => match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("{}: No such file or cannot read ({})", path, e);
                return 1;
            }
        },
        None => Vec::new(),
    };
    let mut prefix: Vec<u8> = Vec::new();
    if opts.input_file.is_some() {
        match names::load_names_list(&mut data) {
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
        people.push(Default::default());
    } else if people.is_empty() {
        print!("{}", usage());
        return 0;
    }

    // ---- resolve selected plugins ----------------------------------------
    let mut selected: Vec<Plugin> = match opts.selected {
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
    fmt::mark_ascii_duplicates(&mut selected);

    // ---- auto cross product ----------------------------------------------
    //
    // Do not materialize the given x family product. With the default Facebook
    // dictionaries that is 10^8 `Person` records (200M+ heap allocations)
    // before the first candidate is generated. The work is streamed from the
    // two dictionaries instead, and parallelized over the family range.
    let mut auto_given: Vec<Option<Vec<u8>>> = Vec::new();
    let mut auto_family: Vec<Option<Vec<u8>>> = Vec::new();
    let is_auto = opts.no_input_names && opts.format_to_recognise.is_none();
    if is_auto {
        auto_family = match read_auto_lines(&opts.family_names) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        };
        auto_given = match read_auto_lines(&opts.given_names) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        };
    }
    let work = if is_auto {
        Work::Auto {
            head: &people,
            names: AutoNames {
                given: &auto_given,
                family: &auto_family,
            },
        }
    } else {
        Work::Fixed(&people)
    };

    // ---- generate ---------------------------------------------------------
    let ctx = Ctx::new(opts.given_names.clone(), opts.family_names.clone());
    let gen_opts = GenOpts {
        substitute: opts.substitute,
        max_sub: opts.max_sub,
        verbose: opts.verbose,
        case_insensitive: opts.case_insensitive,
        suffix: opts.suffix.clone(),
    };

    let est = match &work {
        Work::Fixed(people) => engine::estimate(people, &selected, &ctx, &gen_opts),
        Work::Auto { head, names } => {
            engine::estimate_auto(head, *names, &selected, &ctx, &gen_opts)
        }
    };
    let threads = std::env::var("USERNAME_ANARCHY_THREADS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or_else(|| {
            let available = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1);
            // A short job cannot amortize one thread per CPU. The explicit
            // environment override still permits any requested worker count.
            available.min(est.div_ceil(100_000).max(2) as usize)
        });
    // Allow callers to select a worker count or force serial execution.
    let min_parallel: u64 = std::env::var("USERNAME_ANARCHY_MIN_PARALLEL")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(10_000);
    let parallel = est > min_parallel && threads > 1;

    let generated = if parallel {
        run_parallel(work, &selected, &ctx, &gen_opts, threads, est)
    } else {
        run_serial(work, &selected, &ctx, &gen_opts, est)
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

fn fail(msg: &str) -> ! {
    eprintln!("{}", msg);
    std::process::exit(1);
}
