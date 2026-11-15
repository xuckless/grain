//! Microphone capture for takes and calibration, and the live monitor's input (AGENTS.md §9).
//!
//! The cpal input callback mixes each block to mono and pushes it into a lock-free ring; a
//! writer thread drains the ring into memory. The callback never allocates or locks: the block
//! level goes out through an atomic, and the stream time of the first captured sample through a
//! one-slot ring, so the host can line the capture up with the output stream's cue time.

use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
pub use cpal::StreamInstant;
use engine_core::input::{InputState, StreamState};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Ring capacity: 4 s of audio at 48 kHz. The writer drains every few milliseconds.
const RING: usize = 4 * 48_000;
const DRAIN_EVERY: Duration = Duration::from_millis(5);

/// A finished capture.
pub struct Captured {
    /// Mono samples at `sample_rate`.
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    /// Stream time of `samples[0]`.
    pub first: StreamInstant,
    /// Samples lost because the writer fell behind (should be 0).
    pub dropped: u64,
}

/// An open microphone stream, capturing from the moment it starts.
pub struct Capture {
    stream: InputStream,
    level: Arc<AtomicU32>,
    dropped: Arc<AtomicU64>,
    first: rtrb::Consumer<StreamInstant>,
    stop: Arc<AtomicBool>,
    /// Everything captured so far, appended by the writer thread (never the audio callback).
    samples: Arc<Mutex<Vec<f32>>>,
    writer: Option<JoinHandle<()>>,
}

/// Name of the default input device ("" if there is none).
pub fn default_input_name() -> String {
    cpal::default_host()
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default()
}

/// The default input as the UI shows it. On macOS that's the device picked in System Settings ›
/// Sound or Control Center (an iPhone via Continuity included); every stream opens it afresh.
pub fn input_state() -> InputState {
    match default_input_name() {
        name if name.is_empty() => InputState::Unavailable {
            reason: "No microphone found.".into(),
        },
        device => InputState::Ready { device },
    }
}

/// Mono frames handed to `on_block` per call; longer device blocks are split.
const MONO_BLOCK: usize = 1024;
/// How often the stream's thread checks the device (switched, or silent).
const WATCH_EVERY: Duration = Duration::from_millis(500);
/// A running stream that delivers nothing for this long has failed.
const SILENT_AFTER: Duration = Duration::from_secs(2);

/// Why an input stream stopped, as the audio callback can report it: an atomic code, so the
/// callback never allocates or locks (AGENTS.md §9). `NONE` while it runs.
mod failure {
    pub const NONE: u8 = 0;
    pub const PANICKED: u8 = 1;
    pub const DEVICE_ERROR: u8 = 2;
}

/// Shared between the audio callback (atomics only) and the control side.
struct Health {
    failure: AtomicU8,
    /// Mono frames delivered so far.
    frames: AtomicU64,
    /// Set by the stream's own thread (never the callback): the device was switched or went silent.
    watched: Mutex<StreamState>,
}

impl Health {
    fn state(&self) -> StreamState {
        let watched = self.watched.lock().expect("input health").clone();
        if let StreamState::Failed { .. } = watched {
            return watched;
        }
        match self.failure.load(Ordering::Relaxed) {
            failure::NONE => StreamState::Running,
            failure::PANICKED => StreamState::Failed {
                reason: "The microphone stream hit an internal error.".into(),
            },
            _ => StreamState::Failed {
                reason: "The microphone was disconnected or stopped working.".into(),
            },
        }
    }
}

/// Mixes interleaved `data` of `channels` channels to mono in `mono`, `MONO_BLOCK` frames at a
/// time, converting any sample type to f32. A trailing partial frame is ignored; zero channels
/// deliver nothing. Never allocates.
fn downmix<T>(
    data: &[T],
    channels: usize,
    mono: &mut [f32; MONO_BLOCK],
    mut on_block: impl FnMut(&[f32]),
) where
    T: cpal::Sample,
    f32: cpal::FromSample<T>,
{
    if channels == 0 {
        return;
    }
    let scale = 1.0 / channels as f32;
    for block in data.chunks(MONO_BLOCK * channels) {
        let frames = block.len() / channels;
        for (m, frame) in mono.iter_mut().zip(block.chunks_exact(channels)) {
            *m = frame.iter().map(|&v| v.to_sample::<f32>()).sum::<f32>() * scale;
        }
        on_block(&mono[..frames]);
    }
}

/// An open input stream. cpal streams are not `Send` on every platform, so each lives on its
/// own thread until dropped; that thread also watches the device. Shared by [`Capture`] and
/// [`MonitorInput`].
struct InputStream {
    device: String,
    sample_rate: u32,
    health: Arc<Health>,
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl InputStream {
    /// Opens the default input at `rate` (its default rate when `None`), in whatever sample
    /// format the device offers, and calls `on_block(mono frames, info)` from the audio callback.
    /// `on_block` must not allocate or lock. A panic in the callback, a device error, a switch
    /// of the default input or two silent seconds turn [`InputStream::state`] to `Failed`.
    fn open<F>(rate: Option<u32>, on_block: F) -> Result<Self>
    where
        F: FnMut(&[f32], &cpal::InputCallbackInfo) + Send + 'static,
    {
        let device = cpal::default_host()
            .default_input_device()
            .context("no microphone found")?;
        let name = device.name().unwrap_or_default();
        let config = match rate {
            None => device
                .default_input_config()
                .context("the microphone has no usable format")?,
            Some(wanted) => {
                let fits = |c: &cpal::SupportedStreamConfigRange| {
                    c.min_sample_rate().0 <= wanted && wanted <= c.max_sample_rate().0
                };
                let ranges: Vec<_> = device.supported_input_configs()?.filter(fits).collect();
                ranges
                    .iter()
                    .find(|c| c.sample_format() == cpal::SampleFormat::F32)
                    .or_else(|| ranges.first())
                    .map(|c| c.with_sample_rate(cpal::SampleRate(wanted)))
                    .with_context(|| {
                        format!("{name} can't run at {wanted} Hz, the output's rate")
                    })?
            }
        };
        let format = config.sample_format();
        if !matches!(
            format,
            cpal::SampleFormat::F32
                | cpal::SampleFormat::I16
                | cpal::SampleFormat::I32
                | cpal::SampleFormat::U16
        ) {
            bail!("{name} delivers {format} samples, which aren't supported");
        }
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        if channels == 0 {
            bail!("{name} reports no input channels");
        }
        let health = Arc::new(Health {
            failure: AtomicU8::new(failure::NONE),
            frames: AtomicU64::new(0),
            watched: Mutex::new(StreamState::Running),
        });
        let (shutdown, shutdown_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let stream_config: cpal::StreamConfig = config.into();
        let (cb_health, err_health, watch_health) =
            (health.clone(), health.clone(), health.clone());
        let watched_name = name.clone();
        let mut on_block = on_block;
        let thread = std::thread::spawn(move || {
            // Mixed to mono in a fixed buffer, so the callback never allocates.
            let mut mono = [0.0f32; MONO_BLOCK];
            let callback = move |data: &cpal::Data, info: &cpal::InputCallbackInfo| {
                // Last line of defence: nothing may unwind into CoreAudio (AGENTS.md §9).
                let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut deliver = |m: &[f32]| {
                        cb_health
                            .frames
                            .fetch_add(m.len() as u64, Ordering::Relaxed);
                        on_block(m, info);
                    };
                    match data.sample_format() {
                        cpal::SampleFormat::F32 => data
                            .as_slice::<f32>()
                            .map(|d| downmix(d, channels, &mut mono, &mut deliver)),
                        cpal::SampleFormat::I16 => data
                            .as_slice::<i16>()
                            .map(|d| downmix(d, channels, &mut mono, &mut deliver)),
                        cpal::SampleFormat::I32 => data
                            .as_slice::<i32>()
                            .map(|d| downmix(d, channels, &mut mono, &mut deliver)),
                        cpal::SampleFormat::U16 => data
                            .as_slice::<u16>()
                            .map(|d| downmix(d, channels, &mut mono, &mut deliver)),
                        _ => None,
                    };
                }));
                if run.is_err() {
                    cb_health
                        .failure
                        .store(failure::PANICKED, Ordering::Relaxed);
                }
            };
            let on_error = move |_: cpal::StreamError| {
                err_health
                    .failure
                    .store(failure::DEVICE_ERROR, Ordering::Relaxed);
            };
            let stream = device
                .build_input_stream_raw(&stream_config, format, callback, on_error, None)
                .map_err(anyhow::Error::from)
                .and_then(|s| {
                    s.play()?;
                    Ok(s)
                });
            let stream = match stream {
                Ok(s) => s,
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(()));
            watch(&watch_health, &watched_name, &shutdown_rx);
            drop(stream);
        });
        ready_rx.recv().context("input thread exited")??;
        Ok(Self {
            device: name,
            sample_rate,
            health,
            shutdown,
            thread: Some(thread),
        })
    }

    fn state(&self) -> StreamState {
        self.health.state()
    }

    fn close(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Runs on the stream's own thread until shutdown: marks the stream `Failed` once the system's
/// default input is no longer `device` (the user picked another microphone) or nothing arrived
/// for [`SILENT_AFTER`]. The stream keeps running either way; the host decides what to do.
fn watch(health: &Health, device: &str, shutdown: &mpsc::Receiver<()>) {
    let mut last = (0u64, std::time::Instant::now());
    loop {
        match shutdown.recv_timeout(WATCH_EVERY) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            _ => return,
        }
        let frames = health.frames.load(Ordering::Relaxed);
        let failed = if frames != last.0 {
            last = (frames, std::time::Instant::now());
            None
        } else if last.1.elapsed() >= SILENT_AFTER {
            Some(format!("{device} stopped sending audio."))
        } else {
            None
        };
        let now = default_input_name();
        let failed = failed.or_else(|| {
            (now != device).then(|| match now.as_str() {
                "" => format!("{device} was disconnected."),
                other => format!("The microphone changed from {device} to {other}."),
            })
        });
        if let Some(reason) = failed {
            let mut watched = health.watched.lock().expect("input health");
            if *watched == StreamState::Running {
                *watched = StreamState::Failed { reason };
            }
        }
    }
}

impl Drop for InputStream {
    fn drop(&mut self) {
        self.close();
    }
}

/// The microphone for live monitoring (AGENTS.md §7 step 3): mono samples at the output's rate
/// into a ring that the playback callback drains ([`crate::playback::Playback::attach_monitor`]).
pub struct MonitorInput {
    stream: InputStream,
}

/// Monitor ring capacity: ~185 ms at 44.1 kHz; the output callback keeps it nearly empty.
const MONITOR_RING: usize = 8192;

impl MonitorInput {
    /// Whether the microphone is still delivering (see [`InputStream::open`]).
    pub fn state(&self) -> StreamState {
        self.stream.state()
    }

    /// Opens the default input at `sample_rate`; returns it and the ring's reading end.
    pub fn start(sample_rate: u32) -> Result<(Self, rtrb::Consumer<f32>)> {
        let (mut producer, consumer) = rtrb::RingBuffer::<f32>::new(MONITOR_RING);
        let stream = InputStream::open(Some(sample_rate), move |mono, _| {
            // A full ring means the output stopped draining: drop the newest samples.
            for &v in mono {
                if producer.push(v).is_err() {
                    break;
                }
            }
        })?;
        Ok((Self { stream }, consumer))
    }
}

impl Capture {
    /// Opens the default input device and starts capturing.
    pub fn start() -> Result<Self> {
        let (mut producer, consumer) = rtrb::RingBuffer::<f32>::new(RING);
        let (mut first_tx, first) = rtrb::RingBuffer::<StreamInstant>::new(1);
        let level = Arc::new(AtomicU32::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (cb_level, cb_dropped) = (level.clone(), dropped.clone());
        let mut timed = false;
        let stream = InputStream::open(None, move |mono, info| {
            if !timed {
                timed = first_tx.push(info.timestamp().capture).is_ok();
            }
            let mut peak = 0.0f32;
            for &v in mono {
                peak = peak.max(v.abs());
                if producer.push(v).is_err() {
                    cb_dropped.fetch_add(1, Ordering::Relaxed);
                }
            }
            cb_level.store(peak.to_bits(), Ordering::Relaxed);
        })?;

        let writer_stop = stop.clone();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let out = samples.clone();
        let writer = std::thread::spawn(move || {
            let mut consumer = consumer;
            loop {
                let available = consumer.slots();
                if let Ok(chunk) = consumer.read_chunk(available) {
                    out.lock().expect("capture buffer").extend(chunk);
                }
                if writer_stop.load(Ordering::Acquire) && consumer.slots() == 0 {
                    return;
                }
                std::thread::sleep(DRAIN_EVERY);
            }
        });

        Ok(Self {
            stream,
            level,
            dropped,
            first,
            stop,
            samples,
            writer: Some(writer),
        })
    }

    pub fn device(&self) -> &str {
        &self.stream.device
    }

    /// Whether the microphone is still delivering (see [`InputStream::open`]).
    pub fn state(&self) -> StreamState {
        self.stream.state()
    }

    /// Peak of the latest input block, 0–1.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Samples captured from index `from` on (for live pitch while recording).
    pub fn samples_since(&self, from: usize) -> Vec<f32> {
        let samples = self.samples.lock().expect("capture buffer");
        samples.get(from..).map(<[f32]>::to_vec).unwrap_or_default()
    }

    pub fn sample_rate(&self) -> u32 {
        self.stream.sample_rate
    }

    /// Stream time of the first captured sample, once the first block has arrived.
    pub fn first_instant(&self) -> Option<StreamInstant> {
        self.first.peek().ok().copied()
    }

    /// Stops the stream and returns everything captured.
    pub fn finish(mut self) -> Result<Captured> {
        self.stream.close();
        self.stop.store(true, Ordering::Release);
        self.writer
            .take()
            .expect("writer runs until finish")
            .join()
            .map_err(|_| anyhow::anyhow!("capture writer panicked"))?;
        let samples = std::mem::take(&mut *self.samples.lock().expect("capture buffer"));
        let first = self
            .first
            .pop()
            .map_err(|_| anyhow::anyhow!("the microphone delivered no audio"))?;
        Ok(Captured {
            samples,
            sample_rate: self.stream.sample_rate,
            first,
            dropped: self.dropped.load(Ordering::Relaxed),
        })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stream.close();
        self.stop.store(true, Ordering::Release);
        if let Some(w) = self.writer.take() {
            let _ = w.join();
        }
    }
}

/// `later − earlier` in seconds (negative when `later` is earlier).
pub fn seconds_between(later: StreamInstant, earlier: StreamInstant) -> f64 {
    match later.duration_since(&earlier) {
        Some(d) => d.as_secs_f64(),
        None => -earlier
            .duration_since(&later)
            .map_or(0.0, |d| d.as_secs_f64()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mixed<T>(data: &[T], channels: usize) -> Vec<f32>
    where
        T: cpal::Sample,
        f32: cpal::FromSample<T>,
    {
        let mut mono = [0.0; MONO_BLOCK];
        let mut out = Vec::new();
        downmix(data, channels, &mut mono, |m| out.extend_from_slice(m));
        out
    }

    #[test]
    fn converts_every_sample_format_to_f32() {
        assert_eq!(mixed(&[0.5f32, -0.25], 1), vec![0.5, -0.25]);
        assert_eq!(mixed(&[i16::MIN, 0, 16384], 1), vec![-1.0, 0.0, 0.5]);
        assert_eq!(mixed(&[i32::MIN, 0], 1), vec![-1.0, 0.0]);
        // u16 is offset binary: 32768 is silence.
        assert_eq!(mixed(&[0u16, 32768, 49152], 1), vec![-1.0, 0.0, 0.5]);
    }

    #[test]
    fn averages_channels_and_ignores_a_partial_frame() {
        assert_eq!(mixed(&[1.0f32, 0.0, 0.5, 0.5, 0.9], 2), vec![0.5, 0.5]);
    }

    #[test]
    fn zero_channels_or_no_data_deliver_nothing() {
        assert!(mixed(&[1.0f32, 1.0], 0).is_empty());
        assert!(mixed::<f32>(&[], 2).is_empty());
    }

    #[test]
    fn long_blocks_are_split_without_growing_the_buffer() {
        let data = vec![0.25f32; MONO_BLOCK * 2 * 3 + 6];
        let mut mono = [0.0; MONO_BLOCK];
        let mut sizes = Vec::new();
        downmix(&data, 2, &mut mono, |m| sizes.push(m.len()));
        assert_eq!(sizes, vec![MONO_BLOCK, MONO_BLOCK, MONO_BLOCK, 3]);
    }

    /// Opens the system's default input for about a second (needs a microphone and permission):
    /// `cargo test -p engine-native --release default_input_smoke -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn default_input_smoke() {
        let capture = Capture::start().expect("open the default input");
        std::thread::sleep(Duration::from_millis(1100));
        println!(
            "device {:?} at {} Hz, state {:?}",
            capture.device(),
            capture.sample_rate(),
            capture.state()
        );
        let state = capture.state();
        let got = capture.finish().expect("finish");
        println!("received {} samples", got.samples.len());
        assert_eq!(state, StreamState::Running);
        assert!(!got.samples.is_empty());
    }
}
