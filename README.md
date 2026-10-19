<!-- Badge: replace OWNER/REPO after the first push (see CONTRIBUTING.md#publishing). -->
[![CI](https://github.com/OWNER/REPO/actions/workflows/ci.yml/badge.svg)](https://github.com/OWNER/REPO/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/OWNER/REPO?display_name=tag)](https://github.com/OWNER/REPO/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

# username-anarchy - Rust rewrite

A single static Linux binary that reproduces
[`urbanadventurer/username-anarchy`](https://github.com/urbanadventurer/username-anarchy)
(v0.6, Ruby) with byte-for-byte *output-set* parity, then makes it fast.

It is a drop-in replacement: the same options, the same default datasets, the
same quirks - including the ones that look like bugs, because they change the
output. See [Preserved quirks](#preserved-quirks).

| | Rust | Ruby 3.4 |
|---|---|---|
| tiny invocation (`anna key`) | **0.90 ms** | 161 ms |
| 10k-row input file | **72 ms** | 12.4 s |
| 100k-row input file | **0.52 s** | 142 s |
| `--auto` cross-product (1M people) | **101 ms** | 43.1 s |
| 1M substitutions (full) | **0.28 s** | 10.2 s |
| 1M substitutions, `-m 5` | **1.6 ms** | 7.5 s |

Full numbers, methodology and profiling notes: [`PERFORMANCE.md`](PERFORMANCE.md).
Reproduce with `make bench`. The `--auto` row is the 1,000 × 1,000 Facebook
lists (`-f first -m 1`): Rust streams the cross-product instead of materialising
it, so its peak memory stays at a few MiB.

## Build

Requirements: Rust stable, the `x86_64-unknown-linux-musl` target, and (for the
tests/benchmarks) Python 3 and Ruby with the original checkout.

```sh
rustup target add x86_64-unknown-linux-musl
make            # -> dist/username-anarchy (static, stripped) + dist/names/
```

`cargo build --release` alone also produces a fully static binary at
`target/x86_64-unknown-linux-musl/release/username-anarchy-rs`; `.cargo/config.toml`
selects the musl target. No external musl toolchain is needed - modern rustc
ships a self-contained musl and links it with `+crt-static`.

`make native` builds the same binary with `-C target-cpu=native` for the
machine you are on (a measured ~10% on the 100k-row case). It is deliberately
*not* what `make dist` ships: the released binary stays baseline x86-64 so it
runs on older CPUs.

Verify:

```sh
file dist/username-anarchy        # static-pie linked, stripped
readelf -l dist/username-anarchy | grep INTERP   # no output
ldd dist/username-anarchy         # "statically linked"
```

## Usage

Identical to the original:

```sh
./dist/username-anarchy anna key
./dist/username-anarchy --country france --auto
./dist/username-anarchy --input-file names.txt --select-format first.last
./dist/username-anarchy --list-formats
./dist/username-anarchy --recognise j.smith
./dist/username-anarchy -s on -F '%givenname.%surname' anna key
./dist/username-anarchy --help
```

Datasets are **not** baked in. The binary locates the `names/` tree by trying:

1. `$USERNAME_ANARCHY_ROOT` (a checkout root or a directory containing `names/`),
2. the directory next to the executable (so `dist/` is self-contained),
3. the current working directory.

The flag-driven wordlists (`--given-names`, `--family-names`, `--input-file`)
work as before and take priority.

## Tests

```sh
make test        # full parity gate (435 cases)
make selftest    # verifies the harness itself
```

The harness (`tests/parity/run_parity.py`) runs every case through three
programs and compares the **set** of stdout lines (ordering is not part of the
contract):

* the unmodified Ruby script,
* the Ruby script with `tests/parity/oracle_compat.rb` preloaded,
* the Rust binary.

`oracle_compat.rb` restores `Object#=~` (removed in Ruby 3.2). The original
relies on it in its expansion loop, so on Ruby ≥ 3.2 the stock script crashes on
every substitution except `%D`. The shim is the pre-3.2 behaviour the tool was
written for; the harness uses stock Ruby as the oracle wherever it succeeds, and
otherwise the shim, and it cross-checks the two where both succeed.

Coverage includes all 24 format plugins, every valid `--country`, invalid
countries, CRLF/no-trailing-newline/odd-whitespace/blank-line input files,
expansion ordering, `-m`, suffixes, case folding (`-C true`/`false`), non-ASCII
names, `-a`/`--auto`, `--recognise`, `--list-formats`, long-option
abbreviations, custom `-F` formats (including the token-reassembly quirk), error
paths, and 90 seeded fuzz cases.

The oracle runs under `LC_ALL=C.UTF-8`. Ruby's `String#[]` is byte-based under
the C locale (so `"ß"[0..0]` returns the first *byte* of the UTF-8 encoding) and
character-based under any UTF-8 locale; the port implements the character
semantics a normal UTF-8 environment produces.

## Preserved quirks

These are intentional. Removing them would change the output.

* `Name#init` copies the *raw* `firstinitial` column into `middleinitial` and
  `lastinitial` - a copy/paste bug in the original.
* Empty column values become `nil` before anything else, including initials.
* Plugins return `nil` (and are silently skipped) when a substitution token is
  present and `-s` is off.
* Dedup is **global**, not per person or per plugin.
* Case folding and `--suffix` are applied *after* dedup-key construction is
  chosen, but to the final line; the order is reproduced exactly.
* `--recognise` uses the Facebook dataset unless `--given-names`/`--family-names`
  are supplied.
* Expansion is string-based and round-based: a substitution can reassemble a
  token (`-F '%givenname'` with a missing first name yields a literal
  `%givenname` via `%a-z`, which then expands). Custom `-F` formats use a
  faithful round-based engine; the 24 fixed templates use a faster odometer
  because they provably never reassemble.
* `firstlast[8]` / `first[4]last[4]` bypass the format engine entirely (no
  space/apostrophe stripping, no substitution).
* `-f` is validated against currently registered plugins, and `OptionParser`'s
  `Array` coercion turns `-f ''` and `-f ','` into an empty selection.

## Intentional differences

Documented rather than hidden:

* **Exit codes.** Error paths exit non-zero. Stock Ruby often exits 0 because
  its `error()` helper calls bare `exit`, and its `OptionParser` rescue path
  prints usage and exits 0. The task specification requires non-zero for errors.
* **Usage/banner text** is not byte-identical (the original picks a random ANSI
  colour for the banner). Option names, defaults and examples are preserved.
* **OptionParser edge cases**: ambiguous abbreviations, needless `=value` and
  invalid values exit 1 here instead of raising with a Ruby backtrace.
* **`--recognise` miss rendering.** When no plugin matches, the original prints
  a nested array containing Ruby `Plugin` objects (`#<Plugin:0x…>`), which is
  inherently non-deterministic; the Rust version prints a stable placeholder.
* **Wordlist without a trailing newline.** Ruby's `readlines.map { downcase!;
  strip! }` inserts `nil` for the last line and later raises. Rust reports the
  same condition as an error and exits non-zero.
* **Parallel comment ordering.** Above ~200k estimated candidates output is
  produced on multiple cores, so the order of `-v` comment lines may differ.
  The set of lines is unchanged.

## Layout

```
src/main.rs      CLI parsing, dataset resolution, orchestration
src/names.rs     Name/Person, input-file parsing, wordlist loading
src/fmt.rs       ABK->format translation, templates, expansion engine
src/engine.rs    generation, global dedup, buffered output
src/util.rs      Ruby-compatible string helpers (+ the one unsafe block)
tests/parity/    435-case oracle harness, fixtures, Ruby compat shim
bench/           benchmark driver + generated data
PERFORMANCE.md   measurements, profiling, optimization notes
```
