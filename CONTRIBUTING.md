# Contributing

Thanks for taking a look. This project has one non-negotiable rule:

> **Behavioural parity with the original Ruby tool comes before speed.** If a
> change alters the *set* of output lines for any invocation, it is a bug - even
> if the original's behaviour looks wrong.

## Ground rules

* Preserve the quirks documented in `README.md` ("Preserved quirks"). They are
  part of the output contract.
* Keep the program safe Rust. `unsafe` is allowed only in an isolated,
  profiler-justified hot spot with a written safety argument (there is currently
  exactly one: `util::push_bytes`).
* Do not bake wordlists into the binary. Datasets stay file-path driven.
* Keep the release profile (`opt-level = 3`, LTO, `codegen-units = 1`).

## Workflow

```sh
cargo fmt
cargo clippy --all-targets --release -- -D warnings
cargo test
cargo build --release
python3 tests/parity/run_parity.py --selftest
python3 tests/parity/run_parity.py
```

The parity gate must be green. It runs the original Ruby script and the Rust
binary over 435 cases and compares output sets; see `tests/parity/run_parity.py`
for how the oracle is chosen (stock Ruby where it works, otherwise the
pre-3.2 `Object#=~` shim).

## Adding a parity case

Add a `(kind, args)` entry to the case list in `tests/parity/run_parity.py`.
Kinds are `names` (compare output sets), `usage` (expect exit 0 and usage text),
`listfmt` (compare format listing), `recognise` (masked `Plugin` objects), and
`error` (expect a non-zero exit). Run `--selftest` afterwards to confirm the
harness still detects a deliberately broken candidate.

## Benchmarks

```sh
make bench
python3 bench/run_bench.py --threads 1   # apples-to-apples single core
```

Record meaningful changes in `PERFORMANCE.md`, including the profile output that
justified them.

## Publishing

The repository is ready to push as-is:

```sh
git init -b main          # already initialised if you cloned
git remote add origin git@github.com:OWNER/REPO.git
git push -u origin main
```

Afterwards, replace `OWNER/REPO` in the badge URLs at the top of `README.md`.

CI (`.github/workflows/ci.yml`) runs on every push and pull request: formatting,
clippy, unit tests, a static-musl build, the static-linking assertions, a smoke
test, and the full 435-case parity gate.

## Releasing

Releases are tag-driven:

```sh
git tag v0.6.0
git push origin v0.6.0
```

`.github/workflows/release.yml` then builds the static binary, verifies it has no
`PT_INTERP`, packages `username-anarchy-<version>-x86_64-unknown-linux-musl.tar.gz`
(binary + `names/` + docs), writes `SHA256SUMS`, and publishes a GitHub Release
with generated notes. The `GITHUB_TOKEN` needs `contents: write` (already set in
the workflow).

The archive is self-contained: `tar xzf … && cd username-anarchy-* && ./username-anarchy anna key`
works because the binary finds `names/` next to itself.
