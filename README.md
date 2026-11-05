# compiler233-dotnet

An experimental Rust-hosted C# syntax frontend and a reproducible syntax-parsing comparison with [Roslyn](https://github.com/dotnet/roslyn). This is an early foundation for a .NET compiler project, not a Roslyn replacement.

## Current scope

- Parse UTF-8 C# files and report tree-sitter syntax errors, with one-based line and **UTF-8 byte** column positions.
- Compare repeated full-file syntax parses of the same clean input against the Roslyn compiler bundled with the installed .NET SDK.
- Rust hosts [tree-sitter](https://github.com/tree-sitter/tree-sitter) and [tree-sitter-c-sharp](https://github.com/tree-sitter/tree-sitter-c-sharp); the latter supplies a generated C parser. This is not a fully Rust-written parser.

No binding, type checking, .NET metadata or IL emission, executable output, incremental IDE services, Visual Basic, Roslyn-compatible API or conformance guarantee exists yet. A clean syntax result does not imply a valid C# program. Language versions, diagnostics, preprocessing, syntax trees and error recovery can differ from Roslyn. Do not use this as a compiler or security validator.

## Build and check

Install Rust and a C toolchain required by tree-sitter. The benchmark also needs the .NET 10 SDK and PowerShell 7.

```powershell
cargo test --locked
cargo build --release --locked
./target/release/compiler233-dotnet check benchmarks/corpus/Example.cs
```

`check` exits 0 for a clean tree, 1 for syntax issues, and 2 for usage or I/O errors.

## Syntax parsing benchmark

```powershell
./benchmarks/run.ps1 -Iterations 500 -Warmup 50
./benchmarks/run.ps1 -Source path/to/valid-file.cs -Iterations 500 -Warmup 50
```

The script builds both Release binaries, reads the same UTF-8 bytes into each process once, warms up, then records each full parse and root diagnostic flag. It verifies input SHA-256 and rejects input with parse issues on either side. It reports sorted samples and nearest-rank p50/p95 in nanoseconds, runtime versions and platform. Keep the complete JSON and source when comparing runs. The Roslyn assemblies come from the selected SDK: changes in SDK can change results.

This is **only a syntax-stage microbenchmark**. The two parsers produce different trees, have different grammar/diagnostic behavior and runtime/memory models, and are run in separate processes. A matching clean flag is not a conformance check. Timing excludes build, process startup, file I/O, type checking, IL generation and execution; allocation, peak memory, cold-start, project-scale and incremental performance are not measured. The tiny default fixture is a smoke test, not a representative performance claim. Use a versioned, representative corpus, multiple independent runs and correctness checks before drawing conclusions.

## Roadmap

1. Define a C# language-version and conformance target; differential test syntax and diagnostics on a licensed, representative corpus.
2. Implement a native Rust syntax frontend and replace the generated-C parser; establish correctness and throughput baselines.
3. Add binding, type checking, reference/metadata loading and diagnostics with conformance tests.
4. Emit verifiable .NET metadata and IL, then run produced assemblies against the .NET runtime.
5. Measure equivalent end-to-end compilation and incremental workloads, memory and latency distributions against a pinned Roslyn version; add IDE/API compatibility only with explicit targets.

License: MIT. Third-party dependencies retain their own licenses.
