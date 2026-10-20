# anarchy-rs

A Rust port of [Username Anarchy](https://github.com/urbanadventurer/username-anarchy)
by Andrew Horton. Generate likely usernames from a person's name, a file of
names or the included country wordlists.

It supports the original command-line options, 24 built-in formats, custom
formats, name substitutions and username format recognition.

## Install

Prebuilt archives for each release are attached to the
[releases page](https://github.com/botnetbuddies/anarchy-rs/releases):
Linux x86-64 (fully static), macOS arm64 and Intel, or Windows x86-64 (.zip).
Extract the archive and keep the executable beside `names/`.

## Build

With Rust stable installed:

```sh
rustup target add x86_64-unknown-linux-musl
make
```

This builds a static x86-64 Linux executable at `dist/anarchy-rs` and copies the
wordlists to `dist/names/`. Keep that directory beside the executable when
installing or moving it. Release archives use the same layout.

To build just the executable, run `cargo build --release`. It will be at
`target/x86_64-unknown-linux-musl/release/anarchy-rs`.

## Usage

```sh
./dist/anarchy-rs Anna Key
./dist/anarchy-rs Anna Boom Key -f first.last,fmlast
./dist/anarchy-rs -i people.csv
./dist/anarchy-rs -F '%i.f%l' -@ @example.com Anna Key
./dist/anarchy-rs -a -c france
./dist/anarchy-rs -l
./dist/anarchy-rs -r j.smith
```

Input files can use spaces, commas or tabs. A header identifies the name parts:

```csv
firstname,middlename,lastname
Anna,Boom,Key
Andrew,,Horton
```

`firstinitial`, `middleinitial` and `lastinitial` are also accepted. Without a
header, use first and last names separated by whitespace.

Output is lowercase with duplicates removed across the whole run. Use `-C false`
to preserve format case. `-s on` enables substitutions for missing name parts
and digit ranges; `-m N` limits results per person and format.

Custom formats accept `%f` (first name), `%l` (last name), `%i.f` (first initial)
and `%F`/`%L` (capitalized names), among other tokens. The original ABK notation
is also supported: `ABK` means first initial, middle initial and last initial.
Use `-l` for built-in format examples and `--help` for all options.

## Wordlists

The program looks for `names/` beside the executable, then in the working
directory. Set `USERNAME_ANARCHY_ROOT` to a directory containing `names/` to
use another location. `--given-names` and `--family-names` accept custom files.

## Development

Run `make check` for formatting, linting, documentation and tests. `make test`
runs the unit and CLI regression tests; these checks require Python 3.
See [CONTRIBUTING.md](CONTRIBUTING.md) for the source layout and
[compatibility notes](docs/COMPATIBILITY.md) for behavior inherited from the original.

MIT licensed. Credits and wordlist sources are in
[THIRD_PARTY.md](docs/THIRD_PARTY.md).
