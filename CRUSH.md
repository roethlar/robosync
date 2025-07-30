## Crush.md - Robosync

This file guides agentic coding agents working in the `robosync` repository.

### Development Commands

**Building**
- Dev: `cargo build`
- Release: `cargo build --release`

**Testing**
- All tests: `cargo test`
- Single test module: `cargo test <module_name>`
- With output: `cargo test -- --nocapture`

**Code Quality**
- Format: `cargo fmt`
- Lint: `cargo clippy -- -D warnings`
- Check: `cargo check`

### Code Style

- **Formatting**: Use `cargo fmt` for consistent formatting.
- **Imports**: Group imports logically: `std`, external crates, then project modules.
- **Types**: Use static typing and explicit types where ambiguity is possible.
- **Naming**: Follow Rust conventions (e.g., `snake_case` for variables/functions, `PascalCase` for types).
- **Error Handling**: Use `Result` and `?` for error propagation. Implement `thiserror` for custom error types.
- **Logging**: Use the `log` crate for logging.
- **Comments**: Add comments to explain the "why" of complex logic, not the "what."
- **Dependencies**: Use `tokio` for async I/O and `rayon` for data parallelism.
- **Platform-Specific Code**: Use `#[cfg]` attributes for platform-specific code.
