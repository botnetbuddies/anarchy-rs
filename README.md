# anarchy-rs

A Rust port of [Username Anarchy](https://github.com/urbanadventurer/username-anarchy).
Give it a name or a file of names and it generates likely usernames, such as
`anna.key`, `akey` and `annakey`.

It keeps the original flags, 24 built-in formats and country wordlists. The
executable is a static Linux binary; Ruby is only needed to run the parity tests.

## Build

Install Rust stable and the musl target, then build:

```sh
rustup target add x86_64-unknown-linux-musl
make
```

This creates `dist/username-anarchy` and copies the wordlists into `dist/names/`.
Keep them together when moving the program. For a binary without the bundle,
run `cargo build --release`; the output is
`target/x86_64-unknown-linux-musl/release/username-anarchy-rs`.

`make native` builds a separate version tuned for your CPU. The normal build
stays portable across x86-64 machines.

## Use

```sh
./dist/username-anarchy Anna Key
./dist/username-anarchy Anna Boom Key -f first.last,fmlast
./dist/username-anarchy -i people.csv
./dist/username-anarchy -F '%i.f%l' -@ @example.com Anna Key
./dist/username-anarchy -a -c france
./dist/username-anarchy -l
./dist/username-anarchy -r j.smith
```

Input files can use spaces, commas or tabs. Headers tell the parser which name
parts are available:

```csv
firstname,middlename,lastname
Anna,Boom,Key
Andrew,,Horton
```

`firstinitial`, `middleinitial` and `lastinitial` are also accepted. Without a
header, the original tool's first-name/last-name whitespace layout is used.

Output is lowercase and deduplicated across the whole run. Use `-C false` to
keep format case. `-s on` enables substitutions for missing name parts and digit
ranges; `-m N` limits results per person and format. `--help` lists the options.

Custom formats accept tokens such as `%f` (first name), `%l` (last name), `%i.f`
(first initial) and `%F`/`%L` (capitalized names). You can also use the original
ABK notation: `ABK` means first initial, middle initial, last initial. Run `-l`
to see examples of the built-in formats.

The program finds wordlists through `USERNAME_ANARCHY_ROOT`, beside the
executable, or in the working directory. In a source checkout it also uses
`vendor/username-anarchy/names/` directly. `--given-names` and `--family-names` accept
your own dictionary files.

## Performance

Files are read once, normalized in place where possible, and parsed into
borrowed byte slices. Workers generate numbered batches; one merger applies
global deduplication in input order. Hash tags travel with the candidates, so
the merger does not hash their bytes again.

`USERNAME_ANARCHY_THREADS` sets the worker count. Set it to `1` to force serial
execution. Performance depends on the input and available CPUs; measurements
and the benchmark command are in [PERFORMANCE.md](docs/PERFORMANCE.md).

## Check changes

```sh
make check   # formatting, linting, documentation and tests
make test    # compare against the Ruby original
```

The parity tests compare output sets, including substitutions, Unicode names,
country dictionaries and error cases. Some original quirks affect those outputs;
[COMPATIBILITY.md](docs/COMPATIBILITY.md) documents them. The Ruby source and datasets
are kept in `vendor/username-anarchy/` for testing and distribution.

MIT licensed. See [THIRD_PARTY.md](docs/THIRD_PARTY.md) for attribution.
