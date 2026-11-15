//! Multitrack player: mixer + transport state machine. Runs on the audio thread
//! (cpal callback or AudioWorklet), so `apply`, `render` and `add_track` never allocate
//! (AGENTS.md §9). Besides the song it renders two cues of its own: the count-in click that
//! starts a recording and the calibration chirp ([`crate::cue`]). It also mixes in the
//! singer's own voice when monitoring is on, dry or through live auto-tune ([`LiveTune`]).

use crate::chain::{ChainSettings, VocalChain};
use crate::cue;
use crate::melody::Note;
use crate::psola::LiveTune;
use crate::tune::{self, TuneSettings};
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
    Playing {
        frame: u32,
    },
    Paused {
        frame: u32,
    },
    /// Count-in clicks; playback starts at `frame` when `remaining` reaches zero.
    CountIn {
        frame: u32,
        remaining: u32,
    },
    /// The calibration chirp; the transport stops when it ends.
    Probing {
        remaining: u32,
    },
}

/// Transport state plus the rate its frame counter runs at (the output device rate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TransportSnapshot {
    pub transport: Transport,
    pub sample_rate: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum PlayerCommand {
    Play,
    Pause,
    Stop,
    Seek {
        frame: u32,
    },
    SetMix {
        track: u32,
        params: MixParams,
    },
    /// Clicks `beats` beats of `beat_frames` each, then plays from the current position.
    CountIn {
        beats: u32,
        beat_frames: u32,
    },
    /// Plays the calibration chirp.
    Probe,
    /// What the singer hears of their own voice.
    Monitor {
        mode: MonitorMode,
    },
    /// The live auto-tune settings.
    Tune {
        settings: TuneSettings,
    },
    /// The live vocal chain on the monitored voice (after auto-tune), or none.
    Chain {
        settings: Option<ChainSettings>,
    },
}

/// What the singer hears of their own voice, mixed over the song (headphones only: speakers
/// feed back). The microphone block comes with each [`Player::render`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum MonitorMode {
    Off,
    /// The voice as sung, without added delay.
    Dry,
    /// Through live auto-tune, [`Player::monitor_latency`] late.
    Tuned,
}

/// The monitor as running: its mode and how late the singer hears themself.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MonitorStatus {
    pub mode: MonitorMode,
    /// The devices' round trip (the calibrated latency), plus the shifter's delay when tuned.
    pub latency_ms: f32,
}

impl MonitorStatus {
    /// `shifter_ms`: [`Player::monitor_latency`] in ms; `device_ms`: the calibrated round trip.
    pub fn new(mode: MonitorMode, shifter_ms: f32, device_ms: f32) -> Self {
        let latency_ms = match mode {
            MonitorMode::Off => 0.0,
            MonitorMode::Dry => device_ms,
            MonitorMode::Tuned => device_ms + shifter_ms,
        };
        Self { mode, latency_ms }
    }
}

/// Largest block the monitor processes at once; longer renders are split.
const MONITOR_BLOCK: usize = 1024;

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

#[derive(Debug, Clone)]
pub struct PlayerTrack {
    pub left: Arc<[f32]>,
    pub right: Arc<[f32]>,
    pub mix: MixParams,
}

/// Tracks that can be added after construction (recorded takes) without reallocating.
pub const MAX_ADDED_TRACKS: usize = 16;

/// Click layout of the current or last count-in.
#[derive(Debug, Clone, Copy)]
struct CountInClick {
    beat_frames: u32,
    total: u32,
}

pub struct Player {
    tracks: Vec<PlayerTrack>,
    length: u32,
    transport: Transport,
    sample_rate: u32,
    click: CountInClick,
    chirp: Vec<f32>,
    monitor: MonitorMode,
    tune: LiveTune,
    /// The tuned voice of the current block.
    voice: Vec<f32>,
    /// The live vocal chain, when on ([`PlayerCommand::Chain`]), and its block buffer.
    chain: VocalChain,
    chain_on: bool,
    chained: Vec<f32>,
    /// The reference melody (follow-melody auto-tune), sorted by start.
    reference: Vec<Note>,
}

impl Player {
    /// `sample_rate` is the output rate (cues are synthesized at it). `reference`: the song's
    /// reference melody, for follow-melody auto-tune.
    pub fn new(tracks: Vec<PlayerTrack>, sample_rate: u32, reference: Vec<Note>) -> Self {
        let length = tracks.iter().map(|t| t.left.len()).max().unwrap_or(0) as u32;
        let mut reserved = Vec::with_capacity(tracks.len() + MAX_ADDED_TRACKS);
        reserved.extend(tracks);
        Self {
            tracks: reserved,
            length,
            transport: Transport::Stopped,
            sample_rate,
            click: CountInClick {
                beat_frames: sample_rate,
                total: 0,
            },
            chirp: cue::chirp(sample_rate),
            monitor: MonitorMode::Off,
            tune: LiveTune::new(sample_rate, TuneSettings::default(), MONITOR_BLOCK),
            voice: vec![0.0; MONITOR_BLOCK],
            chain: VocalChain::new(sample_rate),
            chain_on: false,
            chained: vec![0.0; MONITOR_BLOCK],
            reference,
        }
    }

    /// Delay of the tuned monitor, in frames (the device's own buffers come on top).
    pub fn monitor_latency(&self) -> usize {
        self.tune.latency()
    }

    pub fn track_count(&self) -> usize {
        self.tracks.len()
    }

    /// Adds a track (a recorded take) without allocating. When the reserve is used up the
    /// track is handed back, so the caller decides where it is dropped.
    pub fn add_track(&mut self, track: PlayerTrack) -> Result<(), PlayerTrack> {
        if self.tracks.len() == self.tracks.capacity() {
            return Err(track);
        }
        self.length = self.length.max(track.left.len() as u32);
        self.tracks.push(track);
        Ok(())
    }

    /// Swaps track `index`'s audio (a take re-rendered through its chain, AGENTS.md §8.7),
    /// keeping its mix. Returns the old audio so the caller drops it off the audio thread.
    pub fn replace_audio(&mut self, index: usize, audio: Arc<[f32]>) -> Option<Arc<[f32]>> {
        let track = self.tracks.get_mut(index)?;
        self.length = self.length.max(audio.len() as u32);
        track.right = audio.clone();
        Some(std::mem::replace(&mut track.left, audio))
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
            (T::Playing { frame } | T::CountIn { frame, .. }, C::Pause) => T::Paused { frame },
            (
                T::Playing { .. } | T::Paused { .. } | T::CountIn { .. } | T::Probing { .. },
                C::Stop,
            ) => T::Stopped,
            (T::CountIn { .. } | T::Probing { .. }, C::Seek { .. }) => return illegal,
            (_, C::Seek { frame }) if frame >= self.length => return illegal,
            (T::Playing { .. }, C::Seek { frame }) => T::Playing { frame },
            (T::Stopped | T::Paused { .. }, C::Seek { frame }) => T::Paused { frame },
            (_, C::CountIn { beats: 0, .. }) => return illegal,
            (
                T::Stopped | T::Paused { .. } | T::Playing { .. },
                C::CountIn { beats, beat_frames },
            ) => {
                let frame = match self.transport {
                    T::Paused { frame } | T::Playing { frame } => frame,
                    _ => 0,
                };
                let beat_frames = beat_frames.max(1);
                self.click = CountInClick {
                    beat_frames,
                    total: beats.saturating_mul(beat_frames),
                };
                T::CountIn {
                    frame,
                    remaining: self.click.total,
                }
            }
            (T::Stopped | T::Paused { .. }, C::Probe) => T::Probing {
                remaining: self.chirp.len() as u32,
            },
            (state, C::Monitor { mode }) => {
                self.monitor = mode;
                state
            }
            (state, C::Tune { settings }) => {
                self.tune.set_settings(settings);
                state
            }
            (state, C::Chain { settings }) => {
                if let Some(s) = settings {
                    // Switching on starts from silence (no stale reverb tail).
                    if !self.chain_on {
                        self.chain.reset();
                    }
                    self.chain.set(s);
                }
                self.chain_on = settings.is_some();
                state
            }
            (state, C::SetMix { track, params }) => {
                let t = self
                    .tracks
                    .get_mut(track as usize)
                    .ok_or(CommandError::UnknownTrack(track))?;
                t.mix = params;
                state
            }
            (T::Playing { .. } | T::CountIn { .. } | T::Probing { .. }, C::Play)
            | (T::Stopped | T::Paused { .. } | T::Probing { .. }, C::Pause)
            | (T::CountIn { .. } | T::Probing { .. }, C::CountIn { .. })
            | (T::Playing { .. } | T::CountIn { .. } | T::Probing { .. }, C::Probe)
            | (T::Stopped, C::Stop) => return illegal,
        };
        Ok(())
    }

    /// Fills one block of planar stereo output and advances the transport. A block can span
    /// a change of state (the count-in ending, the song ending). `input`: the microphone's
    /// block (mono, same length) for the monitor; empty when there is none.
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32], input: &[f32]) {
        left.fill(0.0);
        right.fill(0.0);
        let n = left.len();
        let seconds = match self.transport {
            Transport::Playing { frame }
            | Transport::Paused { frame }
            | Transport::CountIn { frame, .. } => f64::from(frame) / f64::from(self.sample_rate),
            Transport::Stopped | Transport::Probing { .. } => 0.0,
        };
        let mut i = 0;
        while i < n {
            let done = match self.transport {
                Transport::Playing { frame } => {
                    self.render_song(frame, &mut left[i..], &mut right[i..])
                }
                Transport::CountIn { frame, remaining } => {
                    let k = (remaining as usize).min(n - i);
                    let elapsed = self.click.total - remaining;
                    for j in 0..k {
                        let v = cue::click(
                            elapsed + j as u32,
                            self.click.beat_frames,
                            self.sample_rate,
                        );
                        left[i + j] = v;
                        right[i + j] = v;
                    }
                    let remaining = remaining - k as u32;
                    self.transport = if remaining == 0 {
                        Transport::Playing { frame }
                    } else {
                        Transport::CountIn { frame, remaining }
                    };
                    k
                }
                Transport::Probing { remaining } => {
                    let k = (remaining as usize).min(n - i);
                    let start = self.chirp.len() - remaining as usize;
                    left[i..i + k].copy_from_slice(&self.chirp[start..start + k]);
                    right[i..i + k].copy_from_slice(&self.chirp[start..start + k]);
                    let remaining = remaining - k as u32;
                    self.transport = if remaining == 0 {
                        Transport::Stopped
                    } else {
                        Transport::Probing { remaining }
                    };
                    k
                }
                Transport::Stopped | Transport::Paused { .. } => break,
            };
            i += done.max(1);
        }
        if input.len() == n {
            self.render_monitor(left, right, input, seconds);
        }
        for v in left.iter_mut().chain(right.iter_mut()) {
            *v = v.clamp(-1.0, 1.0);
        }
    }

    /// Mixes the singer's voice into the block (centred). The live tuner keeps running while
    /// the dry voice is heard, so switching to tuned is seamless.
    fn render_monitor(&mut self, left: &mut [f32], right: &mut [f32], input: &[f32], seconds: f64) {
        if self.monitor == MonitorMode::Off {
            return;
        }
        let melody = tune::melody_at(&self.reference, seconds);
        let mut done = 0;
        while done < input.len() {
            let k = (input.len() - done).min(MONITOR_BLOCK);
            let block = &input[done..done + k];
            self.tune.process(block, &mut self.voice[..k], melody);
            let mut heard = match self.monitor {
                MonitorMode::Tuned => &self.voice[..k],
                MonitorMode::Dry | MonitorMode::Off => block,
            };
            if self.chain_on {
                self.chained[..k].copy_from_slice(heard);
                self.chain.process(&mut self.chained[..k]);
                heard = &self.chained[..k];
            }
            for (i, v) in heard.iter().enumerate() {
                left[done + i] += v;
                right[done + i] += v;
            }
            done += k;
        }
    }

    /// Mixes the song from `frame` into `left`/`right`; returns the frames rendered.
    fn render_song(&mut self, frame: u32, left: &mut [f32], right: &mut [f32]) -> usize {
        let start = frame as usize;
        let block = left.len().min((self.length as usize).saturating_sub(start));
        mix_block(
            self.tracks
                .iter()
                .map(|t| (&t.left[..], &t.right[..], t.mix)),
            start,
            &mut left[..block],
            &mut right[..block],
        );
        let next = (start + block) as u32;
        self.transport = if next >= self.length {
            Transport::Stopped
        } else {
            Transport::Playing { frame: next }
        };
        // At the end the transport stops; report the whole block so the caller exits the loop.
        if block == 0 {
            left.len()
        } else {
            block
        }
    }
}

/// The mixing law (AGENTS.md §9): adds every audible track from song frame `start` into
/// `left`/`right` at its gain, with linear pan (centre is full level on both sides). Solo wins
/// over Normal; Muted is silent. Shared by the player and export, so a mixdown is what you hear.
pub fn mix_block<'a>(
    tracks: impl Iterator<Item = (&'a [f32], &'a [f32], MixParams)> + Clone,
    start: usize,
    left: &mut [f32],
    right: &mut [f32],
) {
    let any_solo = tracks.clone().any(|(_, _, m)| m.mode == TrackMode::Soloed);
    for (l, r, mix) in tracks {
        let audible = match mix.mode {
            TrackMode::Soloed => true,
            TrackMode::Normal => !any_solo,
            TrackMode::Muted => false,
        };
        if !audible {
            continue;
        }
        let gl = mix.gain * (1.0 - mix.pan).min(1.0);
        let gr = mix.gain * (1.0 + mix.pan).min(1.0);
        let end = (start + left.len()).min(l.len());
        for (i, s) in (start..end).enumerate() {
            left[i] += gl * l[s];
            right[i] += gr * r[s];
        }
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
        let mut p = Player::new(vec![constant(0.1, 1000)], 1000, Vec::new());
        assert!(p.apply(PlayerCommand::Pause).is_err());
        assert!(p.apply(PlayerCommand::Stop).is_err());
        p.apply(PlayerCommand::Seek { frame: 10 }).unwrap();
        assert_eq!(p.transport(), Transport::Paused { frame: 10 });
        p.apply(PlayerCommand::Play).unwrap();
        assert!(p.apply(PlayerCommand::Play).is_err());
        let (mut l, mut r) = ([0.0; 128], [0.0; 128]);
        p.render(&mut l, &mut r, &[]);
        assert_eq!(p.transport(), Transport::Playing { frame: 138 });
        assert!(p.apply(PlayerCommand::Seek { frame: 1000 }).is_err());
        p.apply(PlayerCommand::Seek { frame: 900 }).unwrap();
        p.render(&mut l, &mut r, &[]);
        assert_eq!(p.transport(), Transport::Stopped);
        assert_eq!(l[99], 0.1);
        assert_eq!(l[100], 0.0);
    }

    #[test]
    fn mix_respects_solo_mute_gain_and_pan() {
        let mut p = Player::new(
            vec![constant(0.1, 256), constant(0.2, 256)],
            1000,
            Vec::new(),
        );
        p.apply(PlayerCommand::Play).unwrap();
        let (mut l, mut r) = ([0.0; 64], [0.0; 64]);
        p.render(&mut l, &mut r, &[]);
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
        p.render(&mut l, &mut r, &[]);
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
        p.render(&mut l, &mut r, &[]);
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
        p.render(&mut l, &mut r, &[]);
        assert!((l[0] - 0.1).abs() < 1e-6);
        assert_eq!(
            p.apply(PlayerCommand::SetMix {
                track: 5,
                params: muted
            }),
            Err(CommandError::UnknownTrack(5))
        );
    }

    #[test]
    fn count_in_clicks_then_plays_from_the_position() {
        let sr = 1000;
        let mut p = Player::new(vec![constant(0.25, 4000)], sr, Vec::new());
        p.apply(PlayerCommand::Seek { frame: 100 }).unwrap();
        p.apply(PlayerCommand::CountIn {
            beats: 2,
            beat_frames: 100,
        })
        .unwrap();
        assert_eq!(
            p.transport(),
            Transport::CountIn {
                frame: 100,
                remaining: 200
            }
        );
        assert!(p.apply(PlayerCommand::Play).is_err());
        assert!(p.apply(PlayerCommand::Seek { frame: 5 }).is_err());
        let (mut l, mut r) = ([0.0; 256], [0.0; 256]);
        p.render(&mut l, &mut r, &[]);
        // 200 frames of click, then the song (0.25) from frame 100 in the same block.
        assert!(l[..200].iter().any(|v| v.abs() > 0.1));
        assert_eq!(l[200], 0.25);
        assert_eq!(p.transport(), Transport::Playing { frame: 156 });
        assert!(p
            .apply(PlayerCommand::CountIn {
                beats: 0,
                beat_frames: 10
            })
            .is_err());
    }

    #[test]
    fn probe_plays_the_chirp_and_stops() {
        let sr = 8000;
        let mut p = Player::new(vec![constant(0.0, 100)], sr, Vec::new());
        p.apply(PlayerCommand::Probe).unwrap();
        assert!(p.apply(PlayerCommand::Pause).is_err());
        let chirp = cue::chirp(sr);
        let mut out = Vec::new();
        let (mut l, mut r) = ([0.0; 128], [0.0; 128]);
        while matches!(p.transport(), Transport::Probing { .. }) {
            p.render(&mut l, &mut r, &[]);
            out.extend_from_slice(&l);
        }
        assert_eq!(&out[..chirp.len()], chirp.as_slice());
        assert_eq!(p.transport(), Transport::Stopped);
    }

    #[test]
    fn added_tracks_play_and_the_reserve_is_bounded() {
        let mut p = Player::new(vec![constant(0.1, 64)], 1000, Vec::new());
        p.add_track(constant(0.2, 64)).unwrap();
        assert_eq!(p.track_count(), 2);
        p.apply(PlayerCommand::Play).unwrap();
        let (mut l, mut r) = ([0.0; 32], [0.0; 32]);
        p.render(&mut l, &mut r, &[]);
        assert!((l[0] - 0.3).abs() < 1e-6);
        for _ in 1..MAX_ADDED_TRACKS {
            p.add_track(constant(0.0, 64)).unwrap();
        }
        assert!(p.add_track(constant(0.0, 64)).is_err());
    }

    #[test]
    fn the_live_chain_shapes_the_monitored_voice() {
        let sr = 44_100;
        let mut p = Player::new(vec![constant(0.0, 100_000)], sr, Vec::new());
        p.apply(PlayerCommand::Monitor {
            mode: MonitorMode::Dry,
        })
        .unwrap();
        let gain = ChainSettings {
            output_db: -6.0206,
            ..ChainSettings::OFF
        };
        p.apply(PlayerCommand::Chain {
            settings: Some(gain),
        })
        .unwrap();
        let voice = vec![0.5f32; 256];
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        p.render(&mut l, &mut r, &voice);
        assert!(l.iter().chain(&r).all(|v| (*v - 0.25).abs() < 1e-4));
        p.apply(PlayerCommand::Chain { settings: None }).unwrap();
        p.render(&mut l, &mut r, &voice);
        assert!(l.iter().all(|v| *v == 0.5), "off: dry again");
    }

    #[test]
    fn monitor_mixes_the_voice_dry_or_tuned() {
        let sr = 44_100;
        let mut p = Player::new(vec![constant(0.0, 100_000)], sr, Vec::new());
        let voice = vec![0.25f32; 512];
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        p.render(&mut l, &mut r, &voice);
        assert!(l.iter().all(|v| *v == 0.0), "off by default");
        p.apply(PlayerCommand::Monitor {
            mode: MonitorMode::Dry,
        })
        .unwrap();
        for _ in 0..4 {
            p.render(&mut l, &mut r, &voice);
            assert!(
                l.iter().chain(&r).all(|v| *v == 0.25),
                "dry: as sung, no delay"
            );
        }
        p.apply(PlayerCommand::Monitor {
            mode: MonitorMode::Tuned,
        })
        .unwrap();
        p.apply(PlayerCommand::Tune {
            settings: TuneSettings {
                mix: 0.0,
                ..TuneSettings::default()
            },
        })
        .unwrap();
        // Tuned at 0% wet: the voice, `monitor_latency` late (the tuner ran during Dry, so
        // its delay line is already full: 2048 samples > the latency).
        p.render(&mut l, &mut r, &voice);
        assert!(l.iter().all(|v| (*v - 0.25).abs() < 1e-6));
        assert_eq!(
            p.transport(),
            Transport::Stopped,
            "monitoring needs no transport"
        );
        assert!(p.monitor_latency() > 0);
    }
}
