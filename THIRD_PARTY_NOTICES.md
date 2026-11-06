# Third-party notices

`compiler233-dotnet` is distributed as a statically linked Rust binary. The
direct dependencies below are compiled into every release artifact; their
licenses apply to those components.

| Crate | Version | License | Upstream |
| --- | --- | --- | --- |
| hao | 0.0.9 | MIT | https://github.com/pigeonhands/hao |
| serde_json | 1.0.154 | MIT OR Apache-2.0 | https://github.com/serde-rs/json |
| sha1 | 0.10.7 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| sha2 | 0.10.9 | MIT OR Apache-2.0 | https://github.com/RustCrypto/hashes |
| tempfile | 3.27.0 | MIT OR Apache-2.0 | https://github.com/Stebalien/tempfile |
| tree-sitter | 0.25.10 | MIT | https://github.com/tree-sitter/tree-sitter |
| tree-sitter-c-sharp | 0.23.5 | MIT | https://github.com/tree-sitter/tree-sitter-c-sharp |

Transitive dependencies are recorded in `Cargo.lock`; each crate's own license
file is available from its crates.io package. Full license texts are reproduced
by the crate authors and are not modified here.

The MIT license text for this project is in `LICENSE`.
