//! Platform-independent engine core.
//!
//! No I/O, no threads, no platform APIs: everything here must build for
//! `wasm32-unknown-unknown` (AGENTS.md §3).

pub mod audio;
pub mod key;
pub mod melody;
pub mod note;
pub mod onset;
pub mod peaks;
pub mod pitch;
pub mod player;
pub mod project;
pub mod resample;
pub mod score;
pub mod stft;
pub mod tempo;

/// Sample rate of every analysis and separation step (AGENTS.md §4.1).
pub const WORKING_SAMPLE_RATE: u32 = 44_100;
