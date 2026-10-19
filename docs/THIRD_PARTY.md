# Third-party material

## Vendored Username Anarchy

`vendor/username-anarchy/` contains the unmodified Ruby tool and its wordlists.
The parity tests use it as their oracle. The release bundle copies its
`names/` directory beside the Rust executable.

- Project: [Username Anarchy](https://github.com/urbanadventurer/username-anarchy)
- Author: Andrew Horton (urbanadventurer)
- Version: 0.6
- Revision: `e0631915ee90afb7ffcabf7900043927ef2f8934`
- License: MIT, in `vendor/username-anarchy/LICENSE`; its notice is also retained
  in the release bundle's `LICENSE`.

The upstream tree keeps its own documentation and packaging files for provenance.
Only its `.git` metadata is omitted.

## Rust dependencies

| Crate | Version | License |
|---|---|---|
| [`rustc-hash`](https://crates.io/crates/rustc-hash) | 2.x | Apache-2.0 OR MIT |

Everything else uses the Rust standard library.

## Test / benchmark tooling (not shipped)

* `hyperfine` (Apache-2.0 OR MIT) - benchmark timing.
* Python 3 (PSF-2.0) - parity harness and benchmark driver.
* Ruby (Ruby license / BSD-2-Clause) - runs the oracle.
