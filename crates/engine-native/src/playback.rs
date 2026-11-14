//! cpal output stream driving an `engine_core::player::Player` (AGENTS.md §9).
//!
//! The control side sends commands and new tracks (takes) through lock-free rings; the audio
//! callback publishes the transport state through an atomic, and the stream time at which each
//! cue (count-in, calibration chirp) starts playing through a ring, so a capture can be lined up
//! with it. The callback never allocates, locks, or drops the last reference to track audio.

use crate::decode;
use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::StreamInstant;
use engine_core::player::{Player, PlayerCommand, PlayerTrack, Transport, MAX_ADDED_TRACKS};
use engine_core::project::{player_tracks, Project, SongAudio};
use engine_core::resample::resample;
use engine_core::take;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

const COMMAND_CAPACITY: usize = 256;
const CUE_CAPACITY: usize = 8;
const BLOCK: usize = 512;

pub struct Playback {
    commands: rtrb::Producer<PlayerCommand>,
    tracks: rtrb::Producer<PlayerTrack>,
    cues: rtrb::Consumer<StreamInstant>,
    transport: Arc<AtomicU64>,
    sample_rate: u32,
    project_rate: u32,
    device: String,
    /// Tracks handed to the callback after it started. Kept here so the audio thread never
    /// frees their audio.
    added: Vec<PlayerTrack>,
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Playback {
    /// Opens the default output device and starts a stopped player with the song's tracks
    /// followed by its takes (`takes[i]` is the timeline audio of `project.takes[i]`, at the
    /// project rate). Audio is resampled to the device rate here, never in the callback.
    pub fn start(project: &Project, audio: &SongAudio, takes: &[Vec<f32>]) -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().context("no output device")?;
        let device_name = device.name().unwrap_or_default();
        let wanted = project.sample_rate;
        let config = device
            .supported_output_configs()?
            .filter(|c| c.sample_format() == cpal::SampleFormat::F32 && c.channels() >= 2)
            .find_map(|c| {
                (c.min_sample_rate().0 <= wanted && wanted <= c.max_sample_rate().0)
                    .then(|| c.with_sample_rate(cpal::SampleRate(wanted)))
            })
            .map(Ok)
            .unwrap_or_else(|| {
                let c = device.default_output_config()?;
                if c.sample_format() != cpal::SampleFormat::F32 {
                    bail!("output device does not support f32 samples");
                }
                Ok(c)
            })?;
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let audio = decode::resample_song(audio, sample_rate)?;
        let mut tracks = player_tracks(project, &audio);
        for (t, timeline) in project.takes.iter().zip(takes) {
            let at_device = resample(&[timeline], project.sample_rate, sample_rate)?.remove(0);
            tracks.push(take::player_track(t, at_device.into()));
        }
        let mut player = Player::new(tracks, sample_rate);

        let (commands, mut consumer) = rtrb::RingBuffer::new(COMMAND_CAPACITY);
        let (track_tx, mut track_rx) = rtrb::RingBuffer::<PlayerTrack>::new(MAX_ADDED_TRACKS);
        let (mut cue_tx, cues) = rtrb::RingBuffer::<StreamInstant>::new(CUE_CAPACITY);
        let transport = Arc::new(AtomicU64::new(encode(Transport::Stopped)));
        let published = transport.clone();
        let (shutdown, shutdown_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let stream_config: cpal::StreamConfig = config.into();

        // cpal streams are not Send on every platform, so the stream lives on its own thread.
        let thread = std::thread::spawn(move || {
            let callback = move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                while let Ok(track) = track_rx.pop() {
                    // The control side checks the reserve, and keeps a reference to the audio.
                    let _ = player.add_track(track);
                }
                let before = player.transport();
                while let Ok(command) = consumer.pop() {
                    // Rejected commands leave the state unchanged; the UI reads the published state.
                    let _ = player.apply(command);
                }
                // A cue starts with the first frame of this buffer.
                if is_cue(player.transport()) && !is_cue(before) {
                    let _ = cue_tx.push(info.timestamp().playback);
                }
                let (mut left, mut right) = ([0.0f32; BLOCK], [0.0f32; BLOCK]);
                for out in data.chunks_mut(BLOCK * channels) {
                    let frames = out.len() / channels;
                    player.render(&mut left[..frames], &mut right[..frames]);
                    for (i, frame) in out.chunks_mut(channels).enumerate() {
                        frame.fill(0.0);
                        frame[0] = left[i];
                        frame[1] = right[i];
                    }
                }
                published.store(encode(player.transport()), Ordering::Release);
            };
            let stream = device.build_output_stream(
                &stream_config,
                callback,
                |e| eprintln!("audio stream error: {e}"),
                None,
            );
            let stream = match stream.map_err(anyhow::Error::from).and_then(|s| {
                s.play()?;
                Ok(s)
            }) {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            let _ = shutdown_rx.recv();
            drop(stream);
        });
        ready_rx.recv().context("audio thread exited")??;
        Ok(Self {
            commands,
            tracks: track_tx,
            cues,
            transport,
            sample_rate,
            project_rate: project.sample_rate,
            device: device_name,
            added: Vec::new(),
            shutdown,
            thread: Some(thread),
        })
    }

    /// Adds take timeline audio (project rate) as a new player track.
    pub fn add_take(&mut self, take: &take::Take, timeline: &[f32]) -> Result<()> {
        if self.added.len() >= MAX_ADDED_TRACKS {
            bail!("too many new takes in one session; save and reopen the song to add more");
        }
        let at_device = resample(&[timeline], self.project_rate, self.sample_rate)?.remove(0);
        let track = take::player_track(take, at_device.into());
        self.added.push(track.clone());
        self.tracks
            .push(track)
            .map_err(|_| anyhow::anyhow!("track queue full"))
    }

    /// Forgets cue times not yet collected (before starting a new cue).
    pub fn clear_cues(&mut self) {
        while self.cues.pop().is_ok() {}
    }

    /// Like [`Playback::take_cue`], without collecting it.
    pub fn peek_cue(&self) -> Option<StreamInstant> {
        self.cues.peek().ok().copied()
    }

    /// Stream time at which the oldest uncollected cue started playing.
    pub fn take_cue(&mut self) -> Option<StreamInstant> {
        self.cues.pop().ok()
    }

    /// Output device name.
    pub fn device(&self) -> &str {
        &self.device
    }

    pub fn send(&mut self, command: PlayerCommand) -> Result<()> {
        self.commands
            .push(command)
            .map_err(|_| anyhow::anyhow!("command queue full"))
    }

    pub fn transport(&self) -> Transport {
        decode_transport(self.transport.load(Ordering::Acquire))
    }

    /// Device sample rate; transport frames are in this rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn is_cue(t: Transport) -> bool {
    matches!(t, Transport::CountIn { .. } | Transport::Probing { .. })
}

/// Bits of the transport word: kind (3) | remaining (29) | frame (32).
const KIND_SHIFT: u32 = 61;
const REMAINING_MASK: u64 = (1 << 29) - 1;

/// Packs a transport state into one atomic word.
fn encode(t: Transport) -> u64 {
    let word = |kind: u64, remaining: u32, frame: u32| {
        (kind << KIND_SHIFT) | ((u64::from(remaining) & REMAINING_MASK) << 32) | u64::from(frame)
    };
    match t {
        Transport::Stopped => 0,
        Transport::Playing { frame } => word(1, 0, frame),
        Transport::Paused { frame } => word(2, 0, frame),
        Transport::CountIn { frame, remaining } => word(3, remaining, frame),
        Transport::Probing { remaining } => word(4, remaining, 0),
    }
}

fn decode_transport(word: u64) -> Transport {
    let frame = word as u32;
    let remaining = ((word >> 32) & REMAINING_MASK) as u32;
    match word >> KIND_SHIFT {
        1 => Transport::Playing { frame },
        2 => Transport::Paused { frame },
        3 => Transport::CountIn { frame, remaining },
        4 => Transport::Probing { remaining },
        _ => Transport::Stopped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_word_round_trip() {
        for t in [
            Transport::Stopped,
            Transport::Playing { frame: 7 },
            Transport::Paused { frame: u32::MAX },
            Transport::CountIn {
                frame: 9,
                remaining: 96_000,
            },
            Transport::Probing { remaining: 24_000 },
        ] {
            assert_eq!(decode_transport(encode(t)), t);
        }
    }
}
