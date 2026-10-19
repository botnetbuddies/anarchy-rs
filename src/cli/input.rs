//! Dataset lookup and automatic-input preparation.

use crate::util;
use std::path::PathBuf;

pub(super) fn root_dir() -> PathBuf {
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
    if std::path::Path::new("vendor/username-anarchy/names").is_dir() {
        if let Ok(cwd) = std::env::current_dir() {
            return cwd.join("vendor/username-anarchy");
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// `File.open(path).readlines.map { |x| x.downcase.strip! }` used by `--auto`.
pub(super) fn read_auto_lines(path: &str) -> Result<Vec<Option<Vec<u8>>>, String> {
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
