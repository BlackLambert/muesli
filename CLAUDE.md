# Muesli

Modular application engine in Rust. Concept and all deviations from it:
[docs/CONCEPT.md](docs/CONCEPT.md).

- Implement milestones in order. Each ends with green `cargo test`,
  `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`.
- Rust edition 2024. `muesli-core` has no dependencies on window, graphics or
  async crates.
- No `unsafe` in the core without a comment justifying it.
- Record deviations from the concept in `docs/CONCEPT.md`.
- Project language is English: code, comments, error messages and docs.
