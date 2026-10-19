//! Username-format recognition and Ruby-compatible result rendering.
use super::{fail, options::Options};
use crate::engine::{self, GenRet, Scratch};
use crate::fmt::{Ctx, Plugin};
use crate::names::{self, Name, Person};
use crate::util;

pub(super) enum RecogResult {
    Found(String),
    Structure(RNode),
}

pub(super) enum RNode {
    Nil,
    Arr(Vec<RNode>),
    Plugins,
}

pub(super) fn render_node(n: &RNode) -> String {
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

pub(super) fn recognise(target: &str, opts: &Options, plugins: &[Plugin]) -> RecogResult {
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
            let mut p: Person<'_> = Default::default();
            p[names::FIRSTNAME] = Some(giv.as_slice());
            p[names::LASTNAME] = Some(fam.as_slice());
            let name = Name::init(&p);
            if opts.verbose {
                println!(
                    "{}",
                    String::from_utf8_lossy(&crate::output::pp_line(&name))
                );
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
