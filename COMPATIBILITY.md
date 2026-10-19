# Compatibility with Username Anarchy

The Rust port follows the output sets produced by Username Anarchy 0.6.
These notes describe the edge cases covered by the Ruby oracle tests.

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
* **Output ordering.** The compatibility gate compares sets of lines. Within
  the Rust port, numbered batches preserve serial generation order, including
  `-v` comments, when running on multiple cores.
