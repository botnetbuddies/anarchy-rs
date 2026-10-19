# Contributing

Preserve the original flags and generated usernames. Add a regression case for
behavior changes and document intentional differences in
[COMPATIBILITY.md](docs/COMPATIBILITY.md).

## Source layout

`src/main.rs` starts the CLI. Each runtime module has one responsibility:

- `cli/`: argument parsing, help, input setup and recognition.
- `names/`: name records, file parsing and dictionary storage.
- `fmt/`: template compilation, resolution and substitution expansion.
- `engine/`: candidate generation, work estimates and ordered worker scheduling.
- `output/`: deduplication, output buffers and verbose formatting.
- `util/`: byte copying, casing and field splitting.

Tests live beside their modules. Keep new code with the responsibility it serves;
avoid adding unrelated behavior to the CLI entry point or generation loop.

## Checks

```sh
make check
make test
USERNAME_ANARCHY_THREADS=4 USERNAME_ANARCHY_MIN_PARALLEL=0 make test
```

`make check` runs formatting, Clippy, documentation checks, Rust and Python
unit tests, and the parity harness self-test. `make test` builds a fresh release binary and runs 451 cases
against the Ruby oracle. The harness uses stock Ruby where it succeeds and the
Ruby 3.2 compatibility shim otherwise; see `tests/parity/oracle_compat.rb`.

Add cases to `tests/parity/run_parity.py`. The comparison kinds are `names`
(output sets), `listfmt` (exact listing), `recognise` (stable object rendering),
`usage` (successful help) and `error` (non-zero exit).

Use safe Rust. The small-copy helper in `src/util/bytes.rs` is the only unsafe
block; it has a written safety argument and profiling evidence. Keep wordlists
external and preserve the portable release profile: optimization level 3, LTO
and one codegen unit.

## Benchmarks

```sh
make bench
python3 bench/run_bench.py --filter tiny
python3 bench/run_bench.py --threads 1
```

Record the input, build settings and before/after measurements in
[PERFORMANCE.md](docs/PERFORMANCE.md). Benchmark timing includes process startup,
input parsing, generation and output writes. Memory is reported when the
installed Hyperfine version provides it.

## Releases

CI runs the checks, builds the static musl binary and verifies static linking.
A `v*` tag triggers `.github/workflows/release.yml`, which packages the binary,
wordlists, README, documentation and license into a release archive. The binary
finds its bundled `names/` directory beside the executable.
