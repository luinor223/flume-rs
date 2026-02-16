# Code Conventions

## Naming

| Item | Style | Example |
|------|-------|---------|
| Functions, variables, modules, files | `snake_case` | `process_record`, `checkpoint_id` |
| Types, traits, enums | `CamelCase` | `StreamElement`, `Operator`, `FlumeError` |
| Constants, statics | `SCREAMING_SNAKE_CASE` | `MAX_BUFFER_SIZE`, `DEFAULT_PARALLELISM` |
| Type parameters | Single uppercase or short `CamelCase` | `T`, `K`, `V`, `In`, `Out` |
| Crate names | `kebab-case` (Cargo) / `snake_case` (code) | `flume-core` / `flume_core` |

## Module Style

```
src/
├── lib.rs
├── types.rs          # Module declaration + contents
├── types/            # Submodules (if needed)
│   ├── record.rs
│   └── stream_element.rs
├── traits.rs
└── error.rs
```

## Re-exports

Flat re-export public API from `lib.rs`. Users should not reach into submodules:

```rust
// lib.rs
mod types;
mod traits;
mod error;

pub use types::{Record, StreamElement, Watermark};
pub use traits::{Operator, Source, Sink};
pub use error::FlumeError;
```

```rust
// User code
use flume_core::{Record, Operator, FlumeError};
```

## Visibility

- Default to private.
- `pub` only for items that are part of the crate's public API.
- `pub(crate)` for internal sharing between modules.
- Never `pub` a field unless there's a good reason — prefer accessor methods or public construction via `new()`.

## Error Handling

Use `thiserror` derive enums. One error type per crate:

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum FlumeError {
    #[error("operator failed: {0}")]
    Operator(String),

    #[error("channel closed")]
    ChannelClosed,

    #[error("checkpoint failed: {0}")]
    Checkpoint(String),
}

pub type FlumeResult<T> = Result<T, FlumeError>;
```

Crate-specific errors in outer crates convert via `#[from]`:

```rust
#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Core(#[from] flume_core::FlumeError),

    #[error("executor failed: {0}")]
    Executor(String),
}
```

## Tests

- **Unit tests**: inline `#[cfg(test)] mod tests` at the bottom of each file. Test private internals here.
- **Integration tests**: `tests/` directory at the crate root. Test only the public API.

```rust
// src/record.rs
pub struct Record<T> { ... }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_creation() { ... }
}
```

## Directory Structure

- Flat for small crates (< ~5 files).
- Nested modules once a crate grows beyond that.
- Maximum 3 levels of nesting (`crate::module::submodule`).
- If you need deeper nesting, split into a separate crate.

## Dependencies

- `flume-core` stays minimal (only `thiserror`). No async runtime, no heavy deps.
- Heavy dependencies (`tokio`, `tonic`, `rdkafka`) only in outer crates.
- Workspace dependency declarations in root `Cargo.toml` for version consistency.

## Documentation

- `//!` inner doc comments at the top of `lib.rs` and module files to describe the module/crate.
- `///` doc comments on all public items (types, traits, functions, fields).
- `//` inline comments only when logic is non-obvious.
- No comments that restate what the code already says.

```rust
//! Core types for the flume-rs stream processing engine.
//!
//! This crate provides foundational types like [`Record`],
//! [`StreamElement`], and the [`Operator`] trait.

/// A single data record in the stream.
pub struct Record<T> { ... }
```

## Formatting & Linting

Enforced via CI:

```bash
cargo fmt --check         # rustfmt.toml: edition 2024, max_width 100
cargo clippy -- -D warnings
```

## General Principles

- Prefer returning `Result` over panicking. Reserve `unwrap()` for tests and provably safe cases.
- Use `#[must_use]` on functions where ignoring the return value is likely a bug.
- Prefer iterators over index-based loops.
- Avoid `clone()` unless necessary — prefer borrowing.
- Keep functions short. If a function exceeds ~50 lines, consider splitting.
