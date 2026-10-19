# CLI regression tests

`cases.json` records commands and expected stdout SHA-256 values. It covers
built-in and custom formats, substitutions, file layouts, Unicode, country
wordlists, recognition, help and invalid arguments. Output checks include line
order and duplicates. Help cases check successful usage output; error cases
require a positive exit status and reject termination by a signal.

The initial expectations were recorded from the Rust port after validation
against Username Anarchy 0.6 at revision
`e0631915ee90afb7ffcabf7900043927ef2f8934`. They include the intentional
differences documented in [COMPATIBILITY.md](../../docs/COMPATIBILITY.md).

Run `make test`, or select cases directly:

```sh
python3 tests/cli/run.py -k first.last
python3 tests/cli/run.py --binary ./dist/anarchy-rs
```

Arguments containing paths are relative to the repository root. To add a case,
put any small input file in `fixtures/`, choose an unused ID and record the
expected stdout digest with Python's `hashlib.sha256`. Verify expected output
independently before adding or changing it; the runner does not update fixtures.
