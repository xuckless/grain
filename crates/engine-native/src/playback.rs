//! cpal output stream driving an `engine_core::player::Player` (AGENTS.md §9).
//!
//! The control side sends commands through a lock-free ring; the audio callback publishes the
//! transport state through an atomic. The callback never allocates or locks.

use crate::decode;
use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use engine_core::player::{Player, PlayerCommand, Transport};
use engine_core::project::{player_tracks, Project, SongAudio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

const COMMAND_CAPACITY: usize = 256;
const BLOCK: usize = 512;

pub struct Playback {
    commands: rtrb::Producer<PlayerCommand>,
    transport: Arc<AtomicU64>,
    sample_rate: u32,
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl Playback {
    /// Opens the default output device and starts a stopped player.
    /// Track audio is resampled to the device rate here, never in the callback.
    pub fn start(project: &Project, audio: &SongAudio) -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_output_device().context("no output device")?;
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
        let mut player = Player::new(player_tracks(project, &audio));

        let (commands, mut consumer) = rtrb::RingBuffer::new(COMMAND_CAPACITY);
        let transport = Arc::new(AtomicU64::new(encode(Transport::Stopped)));
        let published = transport.clone();
        let (shutdown, shutdown_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let stream_config: cpal::StreamConfig = config.into();

        // cpal streams are not Send on every platform, so the stream lives on its own thread.
        let thread = std::thread::spawn(move || {
            let callback = move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                while let Ok(command) = consumer.pop() {
                    // Rejected commands leave the state unchanged; the UI reads the published state.
                    let _ = player.apply(command);
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
            transport,
            sample_rate,
            shutdown,
            thread: Some(thread),
        })
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

/// Packs a transport state into one atomic word: kind in the high 32 bits, frame in the low.
fn encode(t: Transport) -> u64 {
    match t {
        Transport::Stopped => 0,
        Transport::Playing { frame } => (1 << 32) | frame as u64,
        Transport::Paused { frame } => (2 << 32) | frame as u64,
    }
}

fn decode_transport(word: u64) -> Transport {
    let frame = word as u32;
    match word >> 32 {
        1 => Transport::Playing { frame },
        2 => Transport::Paused { frame },
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
        ] {
            assert_eq!(decode_transport(encode(t)), t);
        }
    }
}
