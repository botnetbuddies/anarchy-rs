# Contributing

Keep the original command-line options and generated usernames compatible with
Username Anarchy. Add a regression case for behavior changes and document
intentional differences in [COMPATIBILITY.md](docs/COMPATIBILITY.md).

## Source layout

`src/main.rs` starts the CLI. The code is grouped by responsibility:

- `cli/`: argument parsing, help, input setup and recognition.
- `names/`: name records, file parsing and dictionaries.
- `fmt/`: templates and substitution expansion.
- `engine/`: candidate generation and ordered worker scheduling.
- `output/`: deduplication, output buffers and verbose formatting.
- `util/`: byte copying, casing and field splitting.

Unit tests live beside their modules. The CLI regression cases and input
fixtures live in `tests/cli/`; see its [README](tests/cli/README.md).
Prefer safe Rust and keep any unsafe code small with a written safety argument.

## Checks

Install Rust stable, the `x86_64-unknown-linux-musl` target and Python 3, then run:

```sh
make check
USERNAME_ANARCHY_THREADS=4 USERNAME_ANARCHY_MIN_PARALLEL=0 make test
```

`make check` runs formatting, Clippy, documentation checks and all tests. The
second command also exercises the parallel generation path.

## Releases

A `v*` tag triggers `.github/workflows/release.yml`. It builds the executable
for Linux (static x86-64), macOS (arm64 and Intel) and Windows (x86-64),
packages each one with `names/`, the README, documentation and license, and
publishes them on a GitHub release with a shared `SHA256SUMS`.
Keep wordlists external so users can replace them without rebuilding.
