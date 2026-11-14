//! Microphone capture for takes and calibration (AGENTS.md §9).
//!
//! The cpal input callback mixes each block to mono and pushes it into a lock-free ring; a
//! writer thread drains the ring into memory. The callback never allocates or locks: the block
//! level goes out through an atomic, and the stream time of the first captured sample through a
//! one-slot ring, so the host can line the capture up with the output stream's cue time.

use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::StreamInstant;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
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
    device: String,
    sample_rate: u32,
    level: Arc<AtomicU32>,
    dropped: Arc<AtomicU64>,
    first: rtrb::Consumer<StreamInstant>,
    stop: Arc<AtomicBool>,
    stream_shutdown: mpsc::Sender<()>,
    stream_thread: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<Vec<f32>>>,
}

/// Name of the default input device ("" if there is none).
pub fn default_input_name() -> String {
    cpal::default_host()
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default()
}

impl Capture {
    /// Opens the default input device and starts capturing.
    pub fn start() -> Result<Self> {
        let device = cpal::default_host()
            .default_input_device()
            .context("no microphone found")?;
        let name = device.name().unwrap_or_default();
        let config = device
            .default_input_config()
            .context("the microphone has no usable format")?;
        if config.sample_format() != cpal::SampleFormat::F32 {
            bail!("the microphone does not offer f32 samples");
        }
        let sample_rate = config.sample_rate().0;
        let channels = config.channels() as usize;
        let (mut producer, consumer) = rtrb::RingBuffer::<f32>::new(RING);
        let (mut first_tx, first) = rtrb::RingBuffer::<StreamInstant>::new(1);
        let level = Arc::new(AtomicU32::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (stream_shutdown, shutdown_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let stream_config: cpal::StreamConfig = config.into();
        let (cb_level, cb_dropped) = (level.clone(), dropped.clone());

        let stream_thread = std::thread::spawn(move || {
            let mut timed = false;
            let callback = move |data: &[f32], info: &cpal::InputCallbackInfo| {
                if !timed {
                    timed = first_tx.push(info.timestamp().capture).is_ok();
                }
                let mut peak = 0.0f32;
                for frame in data.chunks(channels) {
                    let v = frame.iter().sum::<f32>() / channels as f32;
                    peak = peak.max(v.abs());
                    if producer.push(v).is_err() {
                        cb_dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
                cb_level.store(peak.to_bits(), Ordering::Relaxed);
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

        let writer_stop = stop.clone();
        let writer = std::thread::spawn(move || {
            let mut consumer = consumer;
            let mut out = Vec::new();
            loop {
                let available = consumer.slots();
                if let Ok(chunk) = consumer.read_chunk(available) {
                    out.extend(chunk);
                }
                if writer_stop.load(Ordering::Acquire) && consumer.slots() == 0 {
                    return out;
                }
                std::thread::sleep(DRAIN_EVERY);
            }
        });

        Ok(Self {
            device: name,
            sample_rate,
            level,
            dropped,
            first,
            stop,
            stream_shutdown,
            stream_thread: Some(stream_thread),
            writer: Some(writer),
        })
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// Peak of the latest input block, 0–1.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Stops the stream and returns everything captured.
    pub fn finish(mut self) -> Result<Captured> {
        self.close_stream();
        self.stop.store(true, Ordering::Release);
        let samples = self
            .writer
            .take()
            .expect("writer runs until finish")
            .join()
            .map_err(|_| anyhow::anyhow!("capture writer panicked"))?;
        let first = self
            .first
            .pop()
            .map_err(|_| anyhow::anyhow!("the microphone delivered no audio"))?;
        Ok(Captured {
            samples,
            sample_rate: self.sample_rate,
            first,
            dropped: self.dropped.load(Ordering::Relaxed),
        })
    }

    fn close_stream(&mut self) {
        let _ = self.stream_shutdown.send(());
        if let Some(t) = self.stream_thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.close_stream();
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
