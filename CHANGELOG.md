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
* `--auto` streams the given × family cross-product instead of materializing it;
  generation starts immediately and peak memory no longer scales with the
  product (1M-person case: 0.14 s / 3.3 MiB vs Ruby 40 s / 259 MiB).
* Output arena stores line terminators, so `finish()` is a move rather than a
  second copy; table growth reuses stored hash tags instead of re-hashing.
* Built-in templates resolve directly to final case-folded form, removing a
  redundant upcase + downcase + buffer copy per candidate.
* Input rows are borrowed (`NameRef::from_person`) instead of cloning six fields
  per row, and the with-substitutions-off skip test is a scan rather than a
  discarded occurrence vector. This also removed the musl allocator-lock
  contention that made the parallel path slower than serial.
* Parity gate grew to 435 cases, including `--auto` case folding and non-ASCII
  names; it now runs the oracle under a UTF-8 locale.
