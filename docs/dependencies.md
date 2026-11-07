# Dependency review

The release checks use the committed `Cargo.lock`. The supported build
toolchain is Rust 1.98.0. This is a tested version, not a claim that every
older Rust toolchain fails.

Compilation and tests do not establish the absence of dependency
vulnerabilities. Recheck advisories and the affected call paths before each
release.

`cargo-about` generates Rust dependency notices from the locked graph.
