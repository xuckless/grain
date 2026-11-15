//! The microphone as a host reports it (AGENTS.md §9). Hosts open the system's default input
//! (on macOS the one picked in System Settings › Sound or Control Center), so the UI only shows
//! which device that is and whether the open stream is still delivering.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The default input device.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum InputState {
    Ready { device: String },
    Unavailable { reason: String },
}

/// An open input stream (a take, a calibration or the live monitor).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum StreamState {
    #[default]
    Running,
    /// It stopped delivering: the device was switched, disconnected or failed.
    Failed { reason: String },
}
