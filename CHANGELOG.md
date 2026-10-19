# Changelog

## Unreleased

- Split the CLI, generation, format, name and output code into focused modules.
- Group supporting documentation under `docs/` and upstream Ruby under `vendor/`.
- Share build logic through the Makefile and support Hyperfine 1.x and 2.x in
  the benchmark helper.

- Compile ordinary ASCII formats into short copy recipes and skip equivalent
  recipes after selection, while retaining verbose format comments.
- Size the default worker count to the estimated job and inline exact short-key
  comparisons in the global deduper.
- Fix Ruby compatibility for Greek case folding, sequential `%%` formats and
  verbose format listings; validate against the broader 747-case Go matrix.

- Import the measured `rust-comp` fixed implementation: borrowed byte parsing,
  required-field masks and ordered worker batches with a single global deduper.
- Reuse worker hash tags during merging and keep 64-bit aggregate output offsets.
- Fix Unicode lowercasing before truncation, retain name columns in wide rows
  and preserve the last value when a header is repeated.
- Disconnect producers on worker errors before joining them.
- Size auto batches and initial table allocations without reserving the full
  Cartesian product.
- Use the `anarchy-rs` banner, remove the top-level `names` symlink and find
  vendored datasets directly from a source checkout.
- Rewrite the README and move compatibility notes into `docs/COMPATIBILITY.md`.

## 0.6.0

Rust port of Username Anarchy 0.6, with the original flags, 24 built-in formats,
custom formats, substitutions, country dictionaries and format recognition.
The default release is a static Linux binary bundled with external wordlists.
