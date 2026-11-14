//! Multitrack player: mixer + transport state machine. Runs on the audio thread
//! (cpal callback or AudioWorklet), so `apply` and `render` never allocate (AGENTS.md §9).

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TrackMode {
    Normal,
    Muted,
    Soloed,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MixParams {
    /// Linear gain, 0..=2.
    pub gain: f32,
    /// -1 = left, 0 = center, 1 = right.
    pub pan: f32,
    pub mode: TrackMode,
}

impl Default for MixParams {
    fn default() -> Self {
        Self {
            gain: 1.0,
            pan: 0.0,
            mode: TrackMode::Normal,
        }
    }
}

/// Transport states. Positions are in frames at the player's sample rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum Transport {
    Stopped,
    Playing { frame: u32 },
    Paused { frame: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum PlayerCommand {
    Play,
    Pause,
    Stop,
    Seek { frame: u32 },
    SetMix { track: u32, params: MixParams },
}

#[derive(Debug, Clone, Copy, PartialEq, thiserror::Error)]
pub enum CommandError {
    #[error("{command:?} is not allowed while {from:?}")]
    IllegalTransition {
        from: Transport,
        command: PlayerCommand,
    },
    #[error("no track {0}")]
    UnknownTrack(u32),
}

pub struct PlayerTrack {
    pub left: Arc<[f32]>,
    pub right: Arc<[f32]>,
    pub mix: MixParams,
}

pub struct Player {
    tracks: Vec<PlayerTrack>,
    length: u32,
    transport: Transport,
}

impl Player {
    pub fn new(tracks: Vec<PlayerTrack>) -> Self {
        let length = tracks.iter().map(|t| t.left.len()).max().unwrap_or(0) as u32;
        Self {
            tracks,
            length,
            transport: Transport::Stopped,
        }
    }

    pub fn transport(&self) -> Transport {
        self.transport
    }

    pub fn length(&self) -> u32 {
        self.length
    }

    pub fn apply(&mut self, command: PlayerCommand) -> Result<(), CommandError> {
        use PlayerCommand as C;
        use Transport as T;
        let illegal = Err(CommandError::IllegalTransition {
            from: self.transport,
            command,
        });
        self.transport = match (self.transport, command) {
            (T::Stopped, C::Play) => T::Playing { frame: 0 },
            (T::Paused { frame }, C::Play) => T::Playing { frame },
            (T::Playing { frame }, C::Pause) => T::Paused { frame },
            (T::Playing { .. } | T::Paused { .. }, C::Stop) => T::Stopped,
            (_, C::Seek { frame }) if frame >= self.length => return illegal,
            (T::Playing { .. }, C::Seek { frame }) => T::Playing { frame },
            (T::Stopped | T::Paused { .. }, C::Seek { frame }) => T::Paused { frame },
            (state, C::SetMix { track, params }) => {
                let t = self
                    .tracks
                    .get_mut(track as usize)
                    .ok_or(CommandError::UnknownTrack(track))?;
                t.mix = params;
                state
            }
            (T::Playing { .. }, C::Play)
            | (T::Stopped | T::Paused { .. }, C::Pause)
            | (T::Stopped, C::Stop) => return illegal,
        };
        Ok(())
    }

    /// Fills one block of planar stereo output and advances the transport.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) {
        left.fill(0.0);
        right.fill(0.0);
        let Transport::Playing { frame } = self.transport else {
            return;
        };
        let any_solo = self.tracks.iter().any(|t| t.mix.mode == TrackMode::Soloed);
        let start = frame as usize;
        let block = left.len().min(self.length as usize - start);
        for track in &self.tracks {
            let audible = match track.mix.mode {
                TrackMode::Soloed => true,
                TrackMode::Normal => !any_solo,
                TrackMode::Muted => false,
            };
            if !audible {
                continue;
            }
            let gl = track.mix.gain * (1.0 - track.mix.pan).min(1.0);
            let gr = track.mix.gain * (1.0 + track.mix.pan).min(1.0);
            let end = (start + block).min(track.left.len());
            for (i, s) in (start..end).enumerate() {
                left[i] += gl * track.left[s];
                right[i] += gr * track.right[s];
            }
        }
        for v in left.iter_mut().chain(right.iter_mut()) {
            *v = v.clamp(-1.0, 1.0);
        }
        let next = (start + block) as u32;
        self.transport = if next >= self.length {
            Transport::Stopped
        } else {
            Transport::Playing { frame: next }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn constant(value: f32, len: usize) -> PlayerTrack {
        PlayerTrack {
            left: vec![value; len].into(),
            right: vec![value; len].into(),
            mix: MixParams::default(),
        }
    }

    #[test]
    fn transport_state_machine() {
        let mut p = Player::new(vec![constant(0.1, 1000)]);
        assert!(p.apply(PlayerCommand::Pause).is_err());
        assert!(p.apply(PlayerCommand::Stop).is_err());
        p.apply(PlayerCommand::Seek { frame: 10 }).unwrap();
        assert_eq!(p.transport(), Transport::Paused { frame: 10 });
        p.apply(PlayerCommand::Play).unwrap();
        assert!(p.apply(PlayerCommand::Play).is_err());
        let (mut l, mut r) = ([0.0; 128], [0.0; 128]);
        p.render(&mut l, &mut r);
        assert_eq!(p.transport(), Transport::Playing { frame: 138 });
        assert!(p.apply(PlayerCommand::Seek { frame: 1000 }).is_err());
        p.apply(PlayerCommand::Seek { frame: 900 }).unwrap();
        p.render(&mut l, &mut r);
        assert_eq!(p.transport(), Transport::Stopped);
        assert_eq!(l[99], 0.1);
        assert_eq!(l[100], 0.0);
    }

    #[test]
    fn mix_respects_solo_mute_gain_and_pan() {
        let mut p = Player::new(vec![constant(0.1, 256), constant(0.2, 256)]);
        p.apply(PlayerCommand::Play).unwrap();
        let (mut l, mut r) = ([0.0; 64], [0.0; 64]);
        p.render(&mut l, &mut r);
        assert!((l[0] - 0.3).abs() < 1e-6);

        let solo = MixParams {
            mode: TrackMode::Soloed,
            ..MixParams::default()
        };
        p.apply(PlayerCommand::SetMix {
            track: 1,
            params: solo,
        })
        .unwrap();
        p.render(&mut l, &mut r);
        assert!((l[0] - 0.2).abs() < 1e-6);

        let panned = MixParams {
            gain: 2.0,
            pan: 1.0,
            mode: TrackMode::Soloed,
        };
        p.apply(PlayerCommand::SetMix {
            track: 1,
            params: panned,
        })
        .unwrap();
        p.render(&mut l, &mut r);
        assert!(l[0].abs() < 1e-6 && (r[0] - 0.4).abs() < 1e-6);

        let muted = MixParams {
            mode: TrackMode::Muted,
            ..MixParams::default()
        };
        p.apply(PlayerCommand::SetMix {
            track: 1,
            params: muted,
        })
        .unwrap();
        p.render(&mut l, &mut r);
        assert!((l[0] - 0.1).abs() < 1e-6);
        assert_eq!(
            p.apply(PlayerCommand::SetMix {
                track: 5,
                params: muted
            }),
            Err(CommandError::UnknownTrack(5))
        );
    }
}
