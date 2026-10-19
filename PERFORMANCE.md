# Performance

The release build uses optimization level 3, LTO, one codegen unit and the
`x86_64-unknown-linux-musl` target. `make native` is available for local CPU
tuning; the distributed build uses the portable target.

## Current implementation

Input files are read into one byte buffer. ASCII names are lowercased in place,
and each record borrows its fields from that buffer. Unicode case conversion
runs before the parser takes those references because it can change byte
lengths. Rows with up to 16 columns use a stack buffer; wider rows fall back to
a growable list rather than losing columns.

Each built-in format records which name fields it requires. With substitutions
off, unavailable formats are skipped before constructing a candidate. A single
field scan per person identifies lowercase ASCII names and names without spaces
or apostrophes, allowing those candidates to avoid repeated case conversion
and cleanup. Custom formats retain the original round-based expansion rules.

Ordinary ASCII formats also have a short recipe containing only field and dot
indexes. When all required fields are present and names contain no spaces,
apostrophes or percent signs, generation can copy those parts without running
the general template interpreter or scanning the candidate for expansion tokens.
Equivalent case-folded recipes are marked after format selection and generated
once per person. Verbose comments remain unchanged. Unicode names, substitutions
and custom formats retain the general paths.

Short deduplication keys use exact, word-sized comparisons in safe Rust. First
and last words overlap to cover all bytes; longer keys use slice equality.

For larger jobs, workers take chunks from an atomic cursor and send numbered
batches through a bounded channel. One merger processes those batches in input
order and deduplicates across the whole run. Workers compute each candidate's
hash once; the merger reuses it. A worker error disconnects the receiver before
joining the other workers, so a blocked producer can exit.

The deduplication table uses linear probing, separate 32-bit hash tags and
64-bit output offsets. Matching tags are checked against the actual bytes;
a hash collision cannot discard a distinct username. The table grows at 70%
occupancy and reuses stored tags when growing. Unique output and line endings
live in a contiguous byte vector, returned without a final flattening copy.

`--auto` walks the two dictionaries without building the Cartesian product of
people. Its initial deduplication table is small and grows with unique output,
since a large product may contain very few distinct usernames. Auto batches
aim for roughly 512 people, with at least one family name per batch.

## Transfer check

Measured on 13th Gen Intel(R) Core(TM) i7-13700K with 24 available CPUs,
`rustc 1.99.0 (b940084d7 2026-09-28)`, and the default portable release profile. The input was
20,000 three-field CSV rows (305,196 bytes).
Each version received five warmups, followed by 50 interleaved, randomly ordered
runs with stdout sent to `/dev/null`. Times include process startup.

| Version | Median | Mean ± standard deviation | Peak RSS (one run) |
|---|---:|---:|---:|
| Previous checkout (`c955cca`) | 33.5 ms | 37.5 ± 11.4 ms | 28.4 MiB |
| Imported fixed implementation | 17.3 ms | 19.0 ± 3.6 ms | 30.4 MiB |

That is about 1.93× faster by median for this input, with a small increase in
peak memory. All three versions—the old checkout, the supplied fixed binary
and the imported code—produced the same 278,205 unique lines. Imported output
also matched the supplied fixed binary byte for byte (2,110,722 bytes).

Input SHA-256: `50856bdff1ab29e5cb31e1538a5db7a983665963e2b01d65722f6806af6b35df`. Output SHA-256:
`231526ff649217964a5f9ae1c04432ec1f04efc739c0bb868d7023f58339a440`.
These measurements describe this workload and machine; they are not a claim
about every input or a general Rust-versus-Go comparison.

## Follow-up optimization

Compared with `d238594` on the same machine and 20k-row CSV, using Hyperfine
with `-N --warmup 50 --runs 500`. Each command writes to `/dev/null`; the build
uses the same portable release profile as before.

| Version | Mean ± standard deviation |
|---|---:|
| Go (`username-anarchy-ng`, comparison binary) | 22.3 ± 4.3 ms |
| Rust before (`d238594`) | 20.0 ± 4.5 ms |
| Rust optimized | 15.4 ± 4.3 ms |

The optimized build used about 23% less wall time than the previous Rust
build in this run (1.30× faster), and was 1.45× faster than the compared Go
binary. Separate single-run peak RSS measurements dropped from
30.9 MiB to 16.3 MiB. Avoiding equivalent recipes also improves the candidate
estimate, preventing an oversized initial deduplication table on this input.
The output SHA-256 is unchanged from the transfer check above.

A larger 200k-row input improved from
227.7 to 207.3 ms median across 20 interleaved process runs per version.
Both versions produced identical bytes. A separate 100-run Hyperfine check with
only `first.last` selected improved from 7.5 to 4.9 ms mean.

Callgrind on the serial 20k-row workload counted 234,850,088 instructions before
and 125,459,977 after (about 47% fewer). The original profile identified template
dispatch, candidate token scans and short-key `memcmp` calls as significant
costs. These are instrumented instruction counts; the table above records
native process timings.

Validation passes all 451 cases in this repository in normal and forced-parallel
mode, plus the Go repository's 747-case live Ruby matrix with exact bytes and
ordering. That broader matrix exposed six pre-existing differences in Greek
case folding, sequential `%%` formatting and verbose format listings; those are
fixed and covered by the Rust harness now. Unit tests check recipe equivalence
with the general engine and every byte position across short-key boundaries.

## Reproducing a measurement

Build both versions with the same Rust toolchain and release profile. Save the
old executable before rebuilding, then run the same input through each:

```sh
hyperfine -N --warmup 5 --runs 50 \
  -n before './before -i people.csv' \
  -n after './target/x86_64-unknown-linux-musl/release/username-anarchy-rs -i people.csv'
```

Compare the outputs as well as the times. The parity harness compares output
sets against Ruby; serial and parallel pipeline tests also check byte-for-byte
ordering across batch boundaries. `python3 bench/run_bench.py` generates larger
inputs and compares with Ruby, including substitution and auto workloads.

`USERNAME_ANARCHY_THREADS=1` forces serial execution. A positive value sets the
worker limit. Without an explicit worker override, Rust budgets one worker per 100,000
estimated candidates, with at least two workers for parallel jobs and up to
the available CPU count. This avoids launching a
thread per CPU for short jobs. The estimate accounts for equivalent ASCII
recipes that generation can omit. The separate parallel threshold is 10,000. `USERNAME_ANARCHY_MIN_PARALLEL` changes that threshold;
setting it to `0` is useful for exercising the worker path in tests.

## Memory limits

The input, deduplication table and complete unique output remain in memory.
Workers also hold candidate batches, and the merger may retain batches that
arrive ahead of their turn. The bounded channel does not cap that reorder
buffer. Large substitution products can therefore still use substantial RAM.
Use format selection, substitution limits or smaller dictionaries to control
the workload. Individual candidates are limited to a 32-bit byte length;
aggregate output offsets are 64-bit.

The only `unsafe` code is the small-copy helper in `src/util.rs`. Its safety
argument is written beside the implementation.
