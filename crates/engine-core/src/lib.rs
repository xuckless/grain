//! Platform-independent engine core.
//!
//! No I/O, no threads, no platform APIs: everything here must build for
//! `wasm32-unknown-unknown` (AGENTS.md §3).

pub mod align;
pub mod audio;
pub mod chain;
pub mod cue;
pub mod drums;
pub mod edit;
pub mod generate;
pub mod harmony;
pub mod key;
pub mod live;
pub mod lyrics;
pub mod melody;
pub mod midi;
pub mod note;
pub mod onset;
pub mod peaks;
pub mod pitch;
pub mod player;
pub mod project;
pub mod psola;
pub mod resample;
pub mod score;
pub mod shift;
pub mod stft;
pub mod stretch;
pub mod take;
pub mod tempo;
pub mod tune;

/// Sample rate of every analysis and separation step (AGENTS.md §4.1).
pub const WORKING_SAMPLE_RATE: u32 = 44_100;
