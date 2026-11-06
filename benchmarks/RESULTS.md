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

## In-memory subset compilation experiment

On the same machine and source hash, three independent runs timed parsing, limited binding and DLL generation. Each process warmed up 15 times and measured 50 compilations; Roslyn used Release optimization. The Rust output was 10,752 bytes and Roslyn output was 9,216 bytes. A separate runtime test loaded the emitted DLL and compared results of all 200 generated methods with Roslyn for one set of inputs per method.

| Run | Rust p50 (ms) | Rust p95 (ms) | Roslyn p50 (ms) | Roslyn p95 (ms) |
| --- | ---: | ---: | ---: | ---: |
| 1 | 0.5208 | 0.5535 | 6.2393 | 8.0329 |
| 2 | 0.5374 | 0.5962 | 6.7005 | 10.1216 |
| 3 | 0.4982 | 0.5804 | 6.2805 | 7.5201 |

Reproduce with `run.ps1 -RustMode Subset -Stage Compile -Source benchmarks/results/generated.cs -Iterations 50 -Warmup 15`, after generating the source with the command above. The timing excludes process startup, file I/O and execution. The compiler supports only a small subset, emits different metadata and omits many checks and features Roslyn performs. The runtime sample is not full correctness or equivalent-work proof; these numbers must not be presented as a full compiler speedup.

## Windows x64 end-to-end supported-subset comparison

On 2026-11-06, Windows 10.0.26300 x64, AMD Ryzen 9 9950X (16 cores, 32 logical processors), Rust 1.99.1 (`x86_64-pc-windows-msvc`) and .NET SDK 10.0.114 with Roslyn 5.0.0-2.26522.126, two synthetic inputs were compiled as libraries by separate compiler processes. Both received the same C# 9 sources, options and `System.Runtime.dll` 10.0.14 reference. The compiler binary was built from `69132038edd3dda341de7e445412bca18c73bf49`; only README/CI files were modified and the benchmark files were untracked in the measuring worktree.

Each workload had 5 warmups followed by 100 alternating runs per compiler. A fresh PE was required on every run. Before timing, both resulting assemblies were executed against a Roslyn oracle: five fixed entry points for both workloads and, for the larger workload, a generated entry point that calls all 200 additional methods and returns 1199. Percentiles use nearest rank. Wall times include process startup, input reads and DLL emission. Memory numbers are per-process Windows peak working set and peak commit charge; they exclude the parent and any child processes.

| Workload | Compiler | p50 ms | p95 ms | p99 ms | Peak working set p50/p95/p99 MiB | Peak commit p50/p95/p99 MiB |
| --- | --- | ---: | ---: | ---: | --- | --- |
| Fixed source | compiler233 | 45.1705 | 50.1468 | 52.3194 | 28.77 / 29.11 / 29.14 | 24.32 / 24.69 / 24.71 |
| Fixed source | Roslyn | 468.6825 | 511.6651 | 575.0524 | 70.95 / 71.16 / 71.45 | 35.84 / 36.00 / 36.45 |
| Fixed source + 200 methods | compiler233 | 50.9856 | 56.0416 | 63.2632 | 31.08 / 31.18 / 31.18 | 26.69 / 26.76 / 26.77 |
| Fixed source + 200 methods | Roslyn | 455.1282 | 485.8671 | 569.5281 | 77.36 / 77.66 / 77.78 | 42.26 / 42.60 / 42.83 |

The median elapsed-time ratios are about 10.38× and 8.93× for these two inputs. These are not full Roslyn performance claims: the inputs cover only a small implemented subset, emitted metadata differs, and the measurement excludes real projects, incremental compilation, analyzers, generators, and most language/API compatibility. OS caches and machine load were not isolated. The source, reference and tool hashes, runtime checks and every raw sample are retained in the [small-workload evidence](evidence/windows-x64-2026-11-06-small.json) and [200-method evidence](evidence/windows-x64-2026-11-06-200-methods.json).

Reproduce on native Windows x64 PowerShell 7 with `./benchmarks/equivalent-compile.ps1 -Iterations 100 -Warmup 5 -ExtraMethods 0` and again with `-ExtraMethods 200`. The reference pack version can be fixed with `-ReferenceVersion 10.0.14`; compare hashes in the generated reports before comparing results.
