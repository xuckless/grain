//! cpal output stream driving an `engine_core::player::Player` (AGENTS.md §9).
//!
//! The control side sends commands and new tracks (takes) through lock-free rings; the audio
//! callback publishes the transport state through an atomic, and the stream time at which each
//! cue (count-in, calibration chirp) starts playing through a ring, so a capture can be lined up
//! with it. The callback never allocates, locks, or drops the last reference to track audio.
//!
//! While monitoring, the microphone's ring ([`crate::capture::MonitorInput`]) is handed to the
//! callback, which mixes the voice in through the player; replaced rings travel back here to be
//! dropped, never on the audio thread.

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
/// Monitor input queued beyond one block is skipped past this, so input and output clocks
/// drifting apart can't build up delay (~12 ms at 44.1 kHz).
const MONITOR_BACKLOG: usize = 512;

/// A change of the callback's microphone feed.
enum MonitorFeed {
    Attach(rtrb::Consumer<f32>),
    Detach,
}

pub struct Playback {
    commands: rtrb::Producer<PlayerCommand>,
    tracks: rtrb::Producer<TrackFeed>,
    /// Audio the callback swapped out ([`Playback::replace_take`]), dropped here.
    old_audio: rtrb::Consumer<Arc<[f32]>>,
    cues: rtrb::Consumer<StreamInstant>,
    transport: Arc<AtomicU64>,
    sample_rate: u32,
    project_rate: u32,
    device: String,
    /// Tracks handed to the callback after it started. Kept here so the audio thread never
    /// frees their audio.
    added: Vec<PlayerTrack>,
    monitor: rtrb::Producer<MonitorFeed>,
    /// Microphone rings the callback let go of, dropped here.
    retired: rtrb::Consumer<rtrb::Consumer<f32>>,
    /// Delay of the tuned monitor (frames at the device rate).
    monitor_latency: usize,
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

/// Track changes handed to the callback.
enum TrackFeed {
    Add(PlayerTrack),
    Replace { index: usize, audio: Arc<[f32]> },
}

/// Swapped-out take audio waiting to be dropped off the audio thread.
const OLD_AUDIO_CAPACITY: usize = 8;

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
        let mut player = Player::new(
            tracks,
            sample_rate,
            project.analysis.reference_notes.clone(),
        );
        let monitor_latency = player.monitor_latency();
        let (monitor, mut monitor_rx) = rtrb::RingBuffer::<MonitorFeed>::new(4);
        let (mut retired_tx, retired) = rtrb::RingBuffer::<rtrb::Consumer<f32>>::new(4);

        let (commands, mut consumer) = rtrb::RingBuffer::new(COMMAND_CAPACITY);
        let (track_tx, mut track_rx) = rtrb::RingBuffer::<TrackFeed>::new(MAX_ADDED_TRACKS);
        let (mut old_audio_tx, old_audio) = rtrb::RingBuffer::<Arc<[f32]>>::new(OLD_AUDIO_CAPACITY);
        let (mut cue_tx, cues) = rtrb::RingBuffer::<StreamInstant>::new(CUE_CAPACITY);
        let transport = Arc::new(AtomicU64::new(encode(Transport::Stopped)));
        let published = transport.clone();
        let (shutdown, shutdown_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let stream_config: cpal::StreamConfig = config.into();

        // cpal streams are not Send on every platform, so the stream lives on its own thread.
        let thread = std::thread::spawn(move || {
            let mut mic: Option<rtrb::Consumer<f32>> = None;
            let callback = move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                while let Ok(feed) = monitor_rx.pop() {
                    let old = match feed {
                        MonitorFeed::Attach(ring) => mic.replace(ring),
                        MonitorFeed::Detach => mic.take(),
                    };
                    if let Some(old) = old {
                        // The control side drains `retired` before every feed change, so it
                        // has room; if not, leaking a ring beats freeing on the audio thread.
                        if let Err(rtrb::PushError::Full(old)) = retired_tx.push(old) {
                            std::mem::forget(old);
                        }
                    }
                }
                while let Ok(feed) = track_rx.pop() {
                    match feed {
                        // The control side checks the reserve, and keeps a reference to the audio.
                        TrackFeed::Add(track) => {
                            let _ = player.add_track(track);
                        }
                        TrackFeed::Replace { index, audio } => {
                            if let Some(old) = player.replace_audio(index, audio) {
                                // Drained before every replace, so there is room; if not,
                                // leaking beats freeing on the audio thread.
                                if let Err(rtrb::PushError::Full(old)) = old_audio_tx.push(old) {
                                    std::mem::forget(old);
                                }
                            }
                        }
                    }
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
                let mut input = [0.0f32; BLOCK];
                for out in data.chunks_mut(BLOCK * channels) {
                    let frames = out.len() / channels;
                    let voice: &[f32] = match mic.as_mut() {
                        Some(ring) => {
                            read_monitor(ring, &mut input[..frames]);
                            &input[..frames]
                        }
                        None => &[],
                    };
                    player.render(&mut left[..frames], &mut right[..frames], voice);
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
            monitor,
            retired,
            old_audio,
            monitor_latency,
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
            .push(TrackFeed::Add(track))
            .map_err(|_| anyhow::anyhow!("track queue full"))
    }

    /// Replaces player track `index`'s audio with `timeline` (project rate): a take
    /// re-rendered through its processing chain (AGENTS.md §8.7). Its mix is kept.
    pub fn replace_take(&mut self, index: usize, timeline: &[f32]) -> Result<()> {
        while self.old_audio.pop().is_ok() {}
        let at_device: Arc<[f32]> = resample(&[timeline], self.project_rate, self.sample_rate)?
            .remove(0)
            .into();
        self.tracks
            .push(TrackFeed::Replace {
                index,
                audio: at_device,
            })
            .map_err(|_| anyhow::anyhow!("track queue full"))
    }

    /// Hands the microphone ring of a [`crate::capture::MonitorInput`] to the callback
    /// (replacing any other).
    pub fn attach_monitor(&mut self, ring: rtrb::Consumer<f32>) -> Result<()> {
        self.feed(MonitorFeed::Attach(ring))
    }

    /// Takes the microphone ring back from the callback.
    pub fn detach_monitor(&mut self) -> Result<()> {
        self.feed(MonitorFeed::Detach)
    }

    fn feed(&mut self, feed: MonitorFeed) -> Result<()> {
        while self.retired.pop().is_ok() {}
        self.monitor
            .push(feed)
            .map_err(|_| anyhow::anyhow!("monitor queue full"))
    }

    /// Delay of the tuned monitor, in ms (the devices' own latency comes on top).
    pub fn monitor_latency_ms(&self) -> f32 {
        self.monitor_latency as f32 * 1000.0 / self.sample_rate as f32
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

/// Fills `out` with the oldest queued microphone samples (silence for any shortfall), first
/// skipping input queued past [`MONITOR_BACKLOG`].
fn read_monitor(ring: &mut rtrb::Consumer<f32>, out: &mut [f32]) {
    let excess = ring.slots().saturating_sub(out.len() + MONITOR_BACKLOG);
    if let Ok(chunk) = ring.read_chunk(excess) {
        chunk.commit_all();
    }
    let n = ring.slots().min(out.len());
    match ring.read_chunk(n) {
        Ok(chunk) => {
            let (a, b) = chunk.as_slices();
            out[..a.len()].copy_from_slice(a);
            out[a.len()..n].copy_from_slice(b);
            chunk.commit_all();
        }
        Err(_) => out[..n].fill(0.0),
    }
    out[n..].fill(0.0);
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

    #[test]
    fn monitor_reads_the_oldest_samples_and_bounds_the_backlog() {
        let (mut tx, mut rx) = rtrb::RingBuffer::<f32>::new(4096);
        let mut out = [1.0f32; 4];
        // Shortfall: two samples queued, the rest silence.
        tx.push(0.5).unwrap();
        tx.push(0.25).unwrap();
        read_monitor(&mut rx, &mut out);
        assert_eq!(out, [0.5, 0.25, 0.0, 0.0]);
        // Too much queued: skip to the newest block + backlog.
        for i in 0..2000 {
            tx.push(i as f32).unwrap();
        }
        read_monitor(&mut rx, &mut out);
        let first = (2000 - 4 - MONITOR_BACKLOG) as f32;
        assert_eq!(out, [first, first + 1.0, first + 2.0, first + 3.0]);
        assert_eq!(rx.slots(), MONITOR_BACKLOG);
    }
}
