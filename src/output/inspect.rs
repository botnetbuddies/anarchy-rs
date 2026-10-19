//! Ruby-compatible verbose name inspection.

use crate::names::NameFields;

/// Ruby `pp [@firstname, @lastname, @firstinitial, @lastinitial, @middlename,
/// @middleinitial]`, printed by `Name#init` in verbose mode.
pub fn pp_line<N: NameFields + ?Sized>(name: &N) -> Vec<u8> {
    let fields = [
        name.firstname(),
        name.lastname(),
        name.firstinitial(),
        name.lastinitial(),
        name.middlename(),
        name.middleinitial(),
    ];
    let mut out = Vec::with_capacity(64);
    out.push(b'[');
    for (i, f) in fields.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b", ");
        }
        match f {
            Some(v) => ruby_inspect(v, &mut out),
            None => out.extend_from_slice(b"nil"),
        }
    }
    out.push(b']');
    out
}

fn ruby_inspect(s: &[u8], out: &mut Vec<u8>) {
    out.push(b'"');
    for &b in s {
        match b {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            0x07 => out.extend_from_slice(b"\\a"),
            0x08 => out.extend_from_slice(b"\\b"),
            0x09 => out.extend_from_slice(b"\\t"),
            0x0a => out.extend_from_slice(b"\\n"),
            0x0b => out.extend_from_slice(b"\\v"),
            0x0c => out.extend_from_slice(b"\\f"),
            0x0d => out.extend_from_slice(b"\\r"),
            0x1b => out.extend_from_slice(b"\\e"),
            0x00 => out.extend_from_slice(b"\\0"),
            _ if b < 0x20 || b == 0x7f => {
                out.extend_from_slice(format!("\\x{:02X}", b).as_bytes());
            }
            _ => out.push(b),
        }
    }
    out.push(b'"');
}
