//! Help text and format examples.
use crate::engine::{self, GenRet, Scratch};
use crate::fmt::{Ctx, Plugin};
use crate::names::{person_of, Name};
use crate::output;

const VERSION: &str = "0.6";

const BANNER: &str = "anarchy-rs\n\n";

pub(super) fn usage() -> String {
    let mut s = String::from(BANNER);
    s.push_str("Usage: anarchy-rs [OPTIONS]... [firstname|first last|first middle last]\n");
    s.push_str(&format!(
        "Based on Username Anarchy by Andrew Horton (urbanadventurer). Version: {}\n\n",
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

pub(super) fn list_plugins(plugins: &[Plugin], verbose: bool) {
    println!("{}\tExample", pad("Plugin name", 20));
    println!("{}", "-".repeat(80));
    let name = Name::init(&person_of("anna", Some("boom"), Some("key")));
    if verbose {
        println!("{}", String::from_utf8_lossy(&output::pp_line(&name)));
    }
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
