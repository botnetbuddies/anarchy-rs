# Changelog

All notable changes to the Rust rewrite. The behavioural contract is the
original tool's output; see `PERFORMANCE.md` for the preserved quirks.

## v0.6.0 — unreleased

Initial release.

* Single static `x86_64-unknown-linux-musl` binary, no runtime dependencies.
* Behavioural parity with `username-anarchy` 0.6, verified by a 400+ case
  oracle harness (`tests/parity/run_parity.py`).
* Same CLI: all short and long options, abbreviation matching, `--country`
  datasets, `--auto`, `--recognise`, `--list-formats`, custom `-F` formats,
  `-m`, suffixes and case folding.
* Faithful expansion engine, including the original's round-based
  token-reassembly behaviour for custom formats.
* Preserved quirks documented in `README.md`, including the `Name#init`
  copy/paste bug and global cross-plugin de-duplication.
* Allocation-free duplicate rejection (arena + open-addressed table), lazy
  `-m` enumeration, lock-free per-worker generation, and an isolated
  profiler-justified `unsafe` small-copy fast path.
