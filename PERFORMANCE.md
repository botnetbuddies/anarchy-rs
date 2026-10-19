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
worker limit. By default Rust uses the available CPU count when the candidate
estimate exceeds 10,000. `USERNAME_ANARCHY_MIN_PARALLEL` changes that threshold;
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
