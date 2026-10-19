
| Case | Rust (min) | Ruby (min) | Speedup | Rust RSS MiB | Ruby RSS MiB |
|---|---:|---:|---:|---:|---:|
| tiny invocation (`anna key`) | 1.0 ms | 161.0 ms | 158× | 3.3 | 14.9 |
| 10k-row input file | 72.0 ms | 12.39 s | 172× | 4.9 | 28.0 |
| 100k-row input file | 516.8 ms | 141.61 s | 274× | 44.4 | 119.8 |
| auto cross-product (1M people) | 101.4 ms | 43.07 s | 425× | 3.4 | 278.4 |
| 1M substitutions (full output) | 277.9 ms | 10.16 s | 37× | 44.7 | 226.8 |
| 1M substitutions, `-m 5` | 1.6 ms | 7.45 s | 4,547× | 3.1 | 167.7 |
| custom `-F` (strict), 10k combinations | 15.1 ms | 431.9 ms | 29× | 3.2 | 19.4 |
