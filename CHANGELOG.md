# Changelog

All notable changes to the Rust rewrite. The behavioural contract is the
original tool's output; see `PERFORMANCE.md` for the preserved quirks.

## v0.6.0 - unreleased

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
* Round 3: one `field_props` scan per person lets plain lowercase ASCII names
  resolve without per-segment case folding; token-free candidates are stripped
  in place and inserted without the `sc.tmp` copy or `emit_lower` guard scan;
  merges reuse each entry's stored hash tag instead of re-hashing; and custom
  `-F` expansion moves finished strings instead of cloning them once per round.
  Measured about 19% faster on 10k rows and 24% on 100k rows/1 thread.
* Round 4: the parallel merge no longer re-deduplicates across rounds -
  `engine::merge` splits entries by their stored hash tag into disjoint
  partitions that are deduplicated in parallel and joined by an offset-shifted
  copy, and `run_parallel` schedules rows dynamically through an atomic
  cursor instead of a static chunk split. The key hash is now selectable at
  build time (`hash-fx`/`hash-wy`/`hash-rapid`, default `hash-fx`); the
  alternatives measured within noise of the default on a busy host, so FxHasher
  stays. About 30% faster on 100k rows/4 threads, at the cost of higher peak
  merge memory (both sides live before the join); `--auto` packing and memory
  are unchanged.
* Benchmarks now include 20k- and 30k-row cases and print rows incrementally.
  `make native` builds an optional `-C target-cpu=native` binary (about 10%
  faster) without changing the portable `dist` artifact.
* Parity gate grew to 435 cases, including `--auto` case folding and non-ASCII
  names; it now runs the oracle under a UTF-8 locale.
