# Changelog

## Unreleased

- Release tags build and publish macOS (arm64 and Intel) and Windows archives
  alongside the static Linux one, with a single SHA256SUMS covering all of them.
- Use `anarchy-rs` consistently for the package, executable and release archives.
- Bundle wordlists in `names/` and make CLI tests self-contained.
- Split argument parsing, generation, formatting and output into focused modules.
- Fix Unicode casing, wide input rows and repeated column headers.

## 0.6.0

Rust port of Username Anarchy 0.6, with the original options, 24 built-in formats,
custom formats, substitutions, country wordlists and username format recognition.
