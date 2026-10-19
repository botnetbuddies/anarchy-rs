//! Owned and borrowed name records, with Ruby initialization semantics.

use crate::util;

pub const FIRSTNAME: usize = 0;
pub const LASTNAME: usize = 1;
pub const FIRSTINITIAL: usize = 2;
pub const LASTINITIAL: usize = 3;
pub const MIDDLENAME: usize = 4;
pub const MIDDLEINITIAL: usize = 5;

/// The six name fields after `Name#init` has run.
#[derive(Clone, Default)]
pub struct Name {
    pub firstname: Option<Vec<u8>>,
    pub lastname: Option<Vec<u8>>,
    pub middlename: Option<Vec<u8>>,
    pub firstinitial: Option<Vec<u8>>,
    pub lastinitial: Option<Vec<u8>>,
    pub middleinitial: Option<Vec<u8>>,
}

/// Read-only view over the six fields, implemented by both the owned [`Name`]
/// and the allocation-free [`NameRef`].
pub trait NameFields {
    fn firstname(&self) -> Option<&[u8]>;
    fn lastname(&self) -> Option<&[u8]>;
    fn middlename(&self) -> Option<&[u8]>;
    fn firstinitial(&self) -> Option<&[u8]>;
    fn lastinitial(&self) -> Option<&[u8]>;
    fn middleinitial(&self) -> Option<&[u8]>;
}

impl NameFields for Name {
    #[inline]
    fn firstname(&self) -> Option<&[u8]> {
        self.firstname.as_deref()
    }
    #[inline]
    fn lastname(&self) -> Option<&[u8]> {
        self.lastname.as_deref()
    }
    #[inline]
    fn middlename(&self) -> Option<&[u8]> {
        self.middlename.as_deref()
    }
    #[inline]
    fn firstinitial(&self) -> Option<&[u8]> {
        self.firstinitial.as_deref()
    }
    #[inline]
    fn lastinitial(&self) -> Option<&[u8]> {
        self.lastinitial.as_deref()
    }
    #[inline]
    fn middleinitial(&self) -> Option<&[u8]> {
        self.middleinitial.as_deref()
    }
}

/// Borrowed equivalent of [`Name`], used by the `--auto` cross-product so that
/// no `Person` or per-field `Vec` is allocated per generated person.
#[derive(Clone, Copy, Default)]
pub struct NameRef<'a> {
    pub firstname: Option<&'a [u8]>,
    pub lastname: Option<&'a [u8]>,
    pub middlename: Option<&'a [u8]>,
    pub firstinitial: Option<&'a [u8]>,
    pub lastinitial: Option<&'a [u8]>,
    pub middleinitial: Option<&'a [u8]>,
    pub present_mask: u8,
}

impl<'a> NameRef<'a> {
    /// Mirror `Name::init` for a person whose only populated columns are
    /// `firstname` and `lastname` (the `--auto` shape): empty values become
    /// `None`, and both initials are the first character of the respective
    /// field.
    #[inline]
    pub fn from_first_last(first: Option<&'a [u8]>, last: Option<&'a [u8]>) -> NameRef<'a> {
        let first = first.filter(|s| !s.is_empty());
        let last = last.filter(|s| !s.is_empty());
        let mut mask = 0u8;
        if first.is_some() {
            mask |= (1 << FIRSTNAME) | (1 << FIRSTINITIAL);
        }
        if last.is_some() {
            mask |= (1 << LASTNAME) | (1 << LASTINITIAL);
        }
        NameRef {
            firstname: first,
            lastname: last,
            firstinitial: first.map(first_char_slice),
            lastinitial: last.map(first_char_slice),
            middlename: None,
            middleinitial: None,
            present_mask: mask,
        }
    }

    /// Borrowing equivalent of [`Name::init`], including the original's
    /// independent-initial columns and its `@middleinitial = s[:firstinitial]`
    /// / `@lastinitial = s[:firstinitial]` copy/paste bug.
    ///
    /// The hot input-file path uses this to avoid six `Vec` clones per row.
    #[inline]
    pub fn from_person(p: &Person<'a>) -> NameRef<'a> {
        let nz = |v: &Option<&'a [u8]>| v.filter(|s| !s.is_empty());
        let firstname = nz(&p[FIRSTNAME]);
        let middlename = nz(&p[MIDDLENAME]);
        let lastname = nz(&p[LASTNAME]);
        let firstinitial_col = nz(&p[FIRSTINITIAL]);
        let firstinitial = firstinitial_col.or_else(|| firstname.map(first_char_slice));
        // `init` derives these from the name, then overwrites them with the
        // *first-initial column* when the respective column is present.
        let middleinitial = if nz(&p[MIDDLEINITIAL]).is_some() {
            firstinitial_col
        } else {
            middlename.map(first_char_slice)
        };
        let lastinitial = if nz(&p[LASTINITIAL]).is_some() {
            firstinitial_col
        } else {
            lastname.map(first_char_slice)
        };
        let mut mask = 0u8;
        if firstname.is_some() {
            mask |= 1 << FIRSTNAME;
        }
        if lastname.is_some() {
            mask |= 1 << LASTNAME;
        }
        if firstinitial.is_some() {
            mask |= 1 << FIRSTINITIAL;
        }
        if lastinitial.is_some() {
            mask |= 1 << LASTINITIAL;
        }
        if middlename.is_some() {
            mask |= 1 << MIDDLENAME;
        }
        if middleinitial.is_some() {
            mask |= 1 << MIDDLEINITIAL;
        }
        NameRef {
            firstname,
            lastname,
            middlename,
            firstinitial,
            middleinitial,
            lastinitial,
            present_mask: mask,
        }
    }
}

#[inline]
fn first_char_slice(s: &[u8]) -> &[u8] {
    &s[..util::nth_char_boundary(s, 1)]
}

impl NameFields for NameRef<'_> {
    #[inline]
    fn firstname(&self) -> Option<&[u8]> {
        self.firstname
    }
    #[inline]
    fn lastname(&self) -> Option<&[u8]> {
        self.lastname
    }
    #[inline]
    fn middlename(&self) -> Option<&[u8]> {
        self.middlename
    }
    #[inline]
    fn firstinitial(&self) -> Option<&[u8]> {
        self.firstinitial
    }
    #[inline]
    fn lastinitial(&self) -> Option<&[u8]> {
        self.lastinitial
    }
    #[inline]
    fn middleinitial(&self) -> Option<&[u8]> {
        self.middleinitial
    }
}

/// A parsed input row: the raw column values *before* `Name#init`.
///
/// `None` means the field was absent or the empty string (the original maps
/// `""` to `nil` before doing anything else).
pub type Person<'a> = [Option<&'a [u8]>; 6];

/// Ruby `String#[]` for a single-character range: `s[0..0]`.
fn first_char(s: &[u8]) -> Vec<u8> {
    let end = util::nth_char_boundary(s, 1);
    s[..end].to_vec()
}

impl Name {
    /// `Name#init`, in the original's exact order.
    ///
    /// Quirk preserved: an explicit `middleinitial`/`lastinitial` column
    /// overwrites the incremented initial with `s[:firstinitial]` (the raw
    /// column value), not with the supplied initial. This is a copy/paste bug
    /// in the original and is reproduced verbatim.
    pub fn init(p: &Person<'_>) -> Name {
        // The original's very first step maps every empty column value to nil.
        let nz = |v: &Option<&[u8]>| -> Option<Vec<u8>> {
            v.filter(|s| !s.is_empty()).map(|s| s.to_vec())
        };
        let p = [
            nz(&p[FIRSTNAME]),
            nz(&p[LASTNAME]),
            nz(&p[FIRSTINITIAL]),
            nz(&p[LASTINITIAL]),
            nz(&p[MIDDLENAME]),
            nz(&p[MIDDLEINITIAL]),
        ];
        let firstname = p[FIRSTNAME].clone();
        let middlename = p[MIDDLENAME].clone();
        let lastname = p[LASTNAME].clone();

        let mut n = Name {
            firstname: firstname.clone(),
            lastname: lastname.clone(),
            middlename: middlename.clone(),
            firstinitial: firstname.as_deref().map(first_char),
            middleinitial: middlename.as_deref().map(first_char),
            lastinitial: lastname.as_deref().map(first_char),
        };

        if let Some(fi) = &p[FIRSTINITIAL] {
            n.firstinitial = Some(fi.clone());
        }
        // Original bug: `@middleinitial = s[:firstinitial]`
        if p[MIDDLEINITIAL].is_some() {
            n.middleinitial = p[FIRSTINITIAL].clone();
        }
        // Original bug: `@lastinitial = s[:firstinitial]`
        if p[LASTINITIAL].is_some() {
            n.lastinitial = p[FIRSTINITIAL].clone();
        }
        n
    }
}

pub fn person_of<'a>(first: &'a str, middle: Option<&'a str>, last: Option<&'a str>) -> Person<'a> {
    let mut p: Person<'a> = Default::default();
    p[FIRSTNAME] = nonempty(first);
    p[MIDDLENAME] = middle.and_then(nonempty);
    p[LASTNAME] = last.and_then(nonempty);
    p
}

fn nonempty(s: &str) -> Option<&[u8]> {
    if s.is_empty() {
        None
    } else {
        Some(s.as_bytes())
    }
}
