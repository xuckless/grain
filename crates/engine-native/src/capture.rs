//! Microphone capture for takes and calibration, and the live monitor's input (AGENTS.md §9).
//!
//! The cpal input callback mixes each block to mono and pushes it into a lock-free ring; a
//! writer thread drains the ring into memory. The callback never allocates or locks: the block
//! level goes out through an atomic, and the stream time of the first captured sample through a
//! one-slot ring, so the host can line the capture up with the output stream's cue time.

use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
pub use cpal::StreamInstant;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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

/// An open input stream. cpal streams are not `Send` on every platform, so each lives on its
/// own thread until dropped. Shared by [`Capture`] and [`MonitorInput`].
struct InputStream {
    device: String,
    sample_rate: u32,
    shutdown: mpsc::Sender<()>,
    thread: Option<JoinHandle<()>>,
}

impl InputStream {
    /// Opens the default input at `rate` (its default rate when `None`) and calls
    /// `on_block(mono frame, info)` from the audio callback for every frame of every block.
    /// `on_block` must not allocate or lock.
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
            Some(wanted) => device
                .supported_input_configs()?
                .filter(|c| c.sample_format() == cpal::SampleFormat::F32)
                .find(|c| c.min_sample_rate().0 <= wanted && wanted <= c.max_sample_rate().0)
                .map(|c| c.with_sample_rate(cpal::SampleRate(wanted)))
                .with_context(|| {
                    format!("the microphone can't run at {wanted} Hz, the output's rate")
                })?,
        };
        if config.sample_format() != cpal::SampleFormat::F32 {
            bail!("the microphone does not offer f32 samples");
        }
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let (shutdown, shutdown_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let stream_config: cpal::StreamConfig = config.into();
        let mut on_block = on_block;
        let thread = std::thread::spawn(move || {
            // Mixed to mono in a fixed buffer, so the callback never allocates.
            let mut mono = [0.0f32; 1024];
            let callback = move |data: &[f32], info: &cpal::InputCallbackInfo| {
                for block in data.chunks(mono.len() * channels) {
                    let frames = block.len() / channels;
                    for (m, frame) in mono.iter_mut().zip(block.chunks(channels)) {
                        *m = frame.iter().sum::<f32>() / channels as f32;
                    }
                    on_block(&mono[..frames], info);
                }
            };
            let stream = device
                .build_input_stream(
                    &stream_config,
                    callback,
                    |e| eprintln!("input stream error: {e}"),
                    None,
                )
                .map_err(anyhow::Error::from)
                .and_then(|s| {
                    s.play()?;
                    Ok(s)
                });
            match stream {
                Ok(s) => {
                    let _ = ready_tx.send(Ok(()));
                    let _ = shutdown_rx.recv();
                    drop(s);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        ready_rx.recv().context("input thread exited")??;
        Ok(Self {
            device: name,
            sample_rate,
            shutdown,
            thread: Some(thread),
        })
    }

    fn close(&mut self) {
        let _ = self.shutdown.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
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
    _stream: InputStream,
}

/// Monitor ring capacity: ~185 ms at 44.1 kHz; the output callback keeps it nearly empty.
const MONITOR_RING: usize = 8192;

impl MonitorInput {
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
        Ok((Self { _stream: stream }, consumer))
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
