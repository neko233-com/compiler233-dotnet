# Local syntax subset experiment

This is an exploratory measurement, not an equivalent-work Roslyn comparison or a full compiler benchmark. The Rust parser implements only the narrow subset documented in the README and creates a different, less complete syntax tree; it does not provide Roslyn's full C# grammar or error recovery.

On Windows x64 with an AMD Ryzen 9 9950X, Rust 1.99.1 and .NET SDK 10.0.114 (Roslyn assembly 5.0.0.0), a generated 200-method, 13,510-byte ASCII file was parsed in three separate runs. Each run used 100 warmups and 300 measured iterations per process. Source SHA-256: `4233e178dfec4f7954a4bce4043dd0acb9861eed9095af54130fb4afb48bc98c`. Times are microseconds, nearest-rank percentiles:

| Run | Rust p50 | Rust p95 | Roslyn p50 | Roslyn p95 |
| --- | ---: | ---: | ---: | ---: |
| 1 | 65.2 | 81.1 | 696.8 | 936.2 |
| 2 | 64.1 | 85.3 | 682.2 | 767.4 |
| 3 | 41.2 | 53.1 | 699.0 | 915.7 |

The large Rust timing variation shows why a single run is not a reliable result. Both programs parsed the same bytes and reported no parse issues. The Rust parser's token and node arenas were retained through timing. Rust was run before Roslyn in separate processes, without CPU isolation; machine load, frequency and JIT/GC can affect results. Input loading, process startup, allocation totals, peak memory, full compilation and executable correctness are outside this measurement. The generated input is repetitive and does not represent a project workload.

Reproduce using `generate-subset.ps1 -Methods 200`, then run `run.ps1 -RustMode Subset -Iterations 300 -Warmup 100` against the generated file. Save the JSON output to keep all latency samples and the input hash. Run `check-subset.ps1` for the small Roslyn differential smoke test. Broader conformance testing and equivalent functionality are prerequisites for a meaningful speedup claim.
