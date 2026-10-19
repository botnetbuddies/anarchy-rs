# Third-party material

## `username-anarchy/` and `names/`

The directory `username-anarchy/` is a vendored copy of the original tool and
its wordlists, used as the **oracle** for the parity test suite. It is included
so that `make test` and CI are fully self-contained and deterministic.

* Project: Username Anarchy
* Author: Andrew Horton (urbanadventurer)
* Version: 0.6 (20 September 2024)
* Source: <https://github.com/urbanadventurer/username-anarchy>
* License: MIT (see `username-anarchy/LICENSE`)

The `.git` metadata was removed when vendoring; the files themselves are
unmodified. `names` at the repository root is a convenience symlink to
`username-anarchy/names`.

## Rust dependencies

| Crate | Version | License |
|---|---|---|
| [`rustc-hash`](https://crates.io/crates/rustc-hash) | 2.x | Apache-2.0 OR MIT |

Everything else uses the Rust standard library.

## Test / benchmark tooling (not shipped)

* `hyperfine` (Apache-2.0 OR MIT) - benchmark timing.
* Python 3 (PSF-2.0) - parity harness and benchmark driver.
* Ruby (Ruby license / BSD-2-Clause) - runs the oracle.
