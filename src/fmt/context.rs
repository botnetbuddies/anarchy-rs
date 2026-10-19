//! Lazy substitution dictionaries and expansion values.
use super::ExpKind;
use crate::names::{load_given_list, WordList};
use crate::util;
use std::sync::OnceLock;

/// Lazily-loaded substitution dictionaries (`given_name_list` / `family_name_list`).
pub struct Ctx {
    pub given_path: String,
    pub family_path: String,
    given: OnceLock<WordList>,
    family: OnceLock<WordList>,
}

fn load_or_die(path: &str) -> WordList {
    match std::fs::read(path) {
        Ok(data) => load_given_list(&data),
        Err(e) => {
            eprintln!("username-anarchy: cannot read file: {path}: {e}");
            std::process::exit(1);
        }
    }
}

impl Ctx {
    pub fn new(given_path: String, family_path: String) -> Ctx {
        Ctx {
            given_path,
            family_path,
            given: OnceLock::new(),
            family: OnceLock::new(),
        }
    }

    #[inline]
    pub fn given(&self) -> &WordList {
        self.given.get_or_init(|| load_or_die(&self.given_path))
    }

    #[inline]
    pub fn family(&self) -> &WordList {
        self.family.get_or_init(|| load_or_die(&self.family_path))
    }

    pub fn readable(path: &str) -> bool {
        std::fs::File::open(path).is_ok()
    }
}

#[inline]
pub fn exp_len(kind: ExpKind, ctx: &Ctx) -> usize {
    match kind {
        ExpKind::AZ | ExpKind::AZUp => 26,
        ExpKind::Given | ExpKind::GivenUp => ctx.given().len(),
        ExpKind::Surname | ExpKind::SurnameUp => ctx.family().len(),
        ExpKind::DD => 100,
        ExpKind::D => 10,
    }
}

#[inline]
pub(super) fn write_exp(kind: ExpKind, i: usize, ctx: &Ctx, out: &mut Vec<u8>) {
    match kind {
        ExpKind::AZ => out.push(b'a' + i as u8),
        ExpKind::AZUp => out.push(b'A' + i as u8),
        ExpKind::Given | ExpKind::GivenUp => util::push_bytes(out, ctx.given().get(i)),
        ExpKind::Surname | ExpKind::SurnameUp => util::push_bytes(out, ctx.family().get(i)),
        ExpKind::DD => {
            out.push(b'0' + (i / 10) as u8);
            out.push(b'0' + (i % 10) as u8);
        }
        ExpKind::D => out.push(b'0' + i as u8),
    }
}
