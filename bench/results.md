
| Case | Rust (min) | Ruby (min) | Speedup | Rust RSS MiB | Ruby RSS MiB |
|---|---:|---:|---:|---:|---:|
| tiny invocation (`anna key`) | 800 µs | 173.7 ms | 217× | 3.3 | 14.9 |
| 10k-row input file | 132.8 ms | 14.37 s | 108× | 5.0 | 28.2 |
| 100k-row input file | 2.59 s | 141.66 s | 55× | 41.3 | 112.9 |
| 1M substitutions (full output) | 427.5 ms | 11.50 s | 27× | 49.1 | 226.7 |
| 1M substitutions, `-m 5` | 2.2 ms | 7.20 s | 3,314× | 3.3 | 167.6 |
| custom `-F` (strict), 10k combinations | 15.9 ms | 376.6 ms | 24× | 3.3 | 19.5 |
