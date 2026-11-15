# Origin

A vendored copy of [`coreaudio-rs`](https://github.com/RustAudio/coreaudio-rs) 0.11.3 from
crates.io (MIT or Apache-2.0, see `LICENSE-MIT` and `LICENSE-APACHE`), used by cpal's macOS
backend through `[patch.crates-io]` in the workspace `Cargo.toml`.

## Changes

Two changes. `Cargo.toml` allows the `mismatched_lifetime_syntaxes` lint (the code predates it). And `src/audio_unit/render_callback.rs` (`input_proc`, the `extern "C"` function
CoreAudio calls for every input and output block): the callback runs inside
`std::panic::catch_unwind`, and a panic fails that render cycle with an error status instead of
unwinding through the FFI boundary. Unwinding there aborts the process (`panic_cannot_unwind`),
which is how Grain crashed on a Continuity (iPhone) microphone. Nothing else differs from the
published 0.11.3.

Drop this copy once an upstream release guards its callbacks the same way.
