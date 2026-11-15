//! Cue signals the player renders on its own: the count-in click before a recording and the
//! calibration chirp, plus the delay estimate that turns a recorded chirp into the round-trip
//! latency (AGENTS.md §9).

use crate::player::PlayerCommand;
use realfft::num_complex::Complex;
use realfft::RealFftPlanner;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A recording's count-in: one bar of 4/4.
pub const COUNT_IN_BEATS: u32 = 4;
/// Tempo range the count-in clicks at (the detected tempo is clamped into it).
const COUNT_IN_BPM: (f64, f64) = (40.0, 240.0);

/// The count-in command for a song at `bpm`, played at `sample_rate`.
pub fn count_in(bpm: f64, sample_rate: u32) -> PlayerCommand {
    let bpm = if bpm.is_finite() {
        bpm.clamp(COUNT_IN_BPM.0, COUNT_IN_BPM.1)
    } else {
        120.0
    };
    PlayerCommand::CountIn {
        beats: COUNT_IN_BEATS,
        beat_frames: (60.0 / bpm * f64::from(sample_rate)).round() as u32,
    }
}

/// Longest round-trip latency a calibration accepts.
pub const MAX_LATENCY_MS: f32 = 1000.0;
/// Calibration peaks below this normalized correlation count as "not heard".
pub const MIN_CONFIDENCE: f32 = 0.2;
/// Capture kept after the chirp ends, so a late echo of it is still recorded.
pub const PROBE_TAIL_SECONDS: f64 = MAX_LATENCY_MS as f64 / 1000.0 + 0.1;

/// How a device pair's round-trip latency was found.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum LatencySource {
    /// Never calibrated: the host's estimate.
    Default,
    /// The loopback chirp test.
    Measured { confidence: f32 },
    /// Set by hand.
    Manual,
}

/// Round-trip latency removed from every recorded take on one input/output device pair.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Latency {
    pub ms: f32,
    pub source: LatencySource,
    /// The device pair it applies to, `input | output`.
    pub devices: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum CalibrationError {
    #[error("the chirp wasn't heard: unplug headphones or hold them to the microphone, and turn the volume up")]
    NotHeard,
    #[error("the measured delay ({0:.0} ms) is out of range")]
    OutOfRange(f32),
}

/// Round-trip latency (ms) and confidence from a calibration capture in which the chirp started
/// playing `cue_at_seconds` into the capture (by the hosts' stream clocks).
pub fn measure_latency(
    capture: &[f32],
    capture_rate: u32,
    cue_at_seconds: f64,
) -> Result<(f32, f32), CalibrationError> {
    let probe = chirp(capture_rate);
    let rate = f64::from(capture_rate);
    let max_lag = ((cue_at_seconds.max(0.0) + f64::from(MAX_LATENCY_MS) / 1000.0) * rate) as usize;
    let found = estimate_delay(capture, &probe, max_lag).ok_or(CalibrationError::NotHeard)?;
    if found.confidence < MIN_CONFIDENCE {
        return Err(CalibrationError::NotHeard);
    }
    let ms = ((found.samples / rate - cue_at_seconds) * 1000.0) as f32;
    if !(0.0..=MAX_LATENCY_MS).contains(&ms) {
        return Err(CalibrationError::OutOfRange(ms));
    }
    Ok((ms, found.confidence))
}

/// Length of the calibration chirp.
pub const CHIRP_SECONDS: f64 = 0.5;
/// Sweep range of the chirp: wide enough for a sharp correlation peak, inside what laptop
/// speakers and microphones reproduce.
const CHIRP_FROM_HZ: f64 = 300.0;
const CHIRP_TO_HZ: f64 = 6000.0;
const CHIRP_FADE_SECONDS: f64 = 0.01;
const CHIRP_GAIN: f32 = 0.5;

/// Count-in click: a short decaying tone, higher on the first beat of the bar.
const CLICK_SECONDS: f64 = 0.06;
const CLICK_DECAY_SECONDS: f64 = 0.012;
const CLICK_DOWNBEAT_HZ: f64 = 1760.0;
const CLICK_BEAT_HZ: f64 = 880.0;
const CLICK_GAIN: f64 = 0.6;

/// Count-in sample `elapsed` frames after the count-in started. Pure, so the audio thread can
/// synthesize it without a buffer.
pub fn click(elapsed: u32, beat_frames: u32, sample_rate: u32) -> f32 {
    let beat_frames = beat_frames.max(1);
    let within = elapsed % beat_frames;
    let t = f64::from(within) / f64::from(sample_rate);
    if t >= CLICK_SECONDS {
        return 0.0;
    }
    let hz = if elapsed < beat_frames {
        CLICK_DOWNBEAT_HZ
    } else {
        CLICK_BEAT_HZ
    };
    (CLICK_GAIN * (std::f64::consts::TAU * hz * t).sin() * (-t / CLICK_DECAY_SECONDS).exp()) as f32
}

/// The calibration chirp at `sample_rate`: an exponential sweep with short fades.
pub fn chirp(sample_rate: u32) -> Vec<f32> {
    let sr = f64::from(sample_rate);
    let n = (CHIRP_SECONDS * sr) as usize;
    let k = (CHIRP_TO_HZ / CHIRP_FROM_HZ).ln();
    let fade = CHIRP_FADE_SECONDS * sr;
    (0..n)
        .map(|i| {
            let t = i as f64 / sr;
            let phase = std::f64::consts::TAU * CHIRP_FROM_HZ * CHIRP_SECONDS / k
                * ((k * t / CHIRP_SECONDS).exp() - 1.0);
            let edge = (i as f64).min((n - 1 - i) as f64);
            let env = if edge < fade {
                0.5 - 0.5 * (std::f64::consts::PI * edge / fade).cos()
            } else {
                1.0
            };
            CHIRP_GAIN * (phase.sin() * env) as f32
        })
        .collect()
}

/// Where a known probe sits inside a recording.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DelayEstimate {
    /// Sample index (fractional) of the probe's first sample in the recording.
    pub samples: f64,
    /// Normalized correlation at the peak, 0–1: how clearly the probe was heard.
    pub confidence: f32,
}

/// Finds `probe` in `recording` at lags `0..=max_lag` by FFT cross-correlation, with parabolic
/// refinement of the peak. `None` when the recording is shorter than the probe.
pub fn estimate_delay(recording: &[f32], probe: &[f32], max_lag: usize) -> Option<DelayEstimate> {
    if probe.is_empty() || recording.len() < probe.len() {
        return None;
    }
    let max_lag = max_lag.min(recording.len() - probe.len());
    let n = (recording.len() + probe.len()).next_power_of_two();
    let mut planner = RealFftPlanner::<f32>::new();
    let forward = planner.plan_fft_forward(n);
    let inverse = planner.plan_fft_inverse(n);
    let spectrum = |signal: &[f32]| -> Vec<Complex<f32>> {
        let mut input = forward.make_input_vec();
        input[..signal.len()].copy_from_slice(signal);
        let mut out = forward.make_output_vec();
        forward
            .process(&mut input, &mut out)
            .expect("buffer sizes come from the plan");
        out
    };
    let r = spectrum(recording);
    let p = spectrum(probe);
    let mut product: Vec<Complex<f32>> = r.iter().zip(&p).map(|(a, b)| a * b.conj()).collect();
    let mut corr = inverse.make_output_vec();
    inverse
        .process(&mut product, &mut corr)
        .expect("buffer sizes come from the plan");

    let (lag, peak) = corr[..=max_lag]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(i, v)| (i, *v))?;
    let refined = if lag > 0 && lag < max_lag {
        let (a, b, c) = (corr[lag - 1], corr[lag], corr[lag + 1]);
        let denom = a - 2.0 * b + c;
        if denom.abs() > f32::EPSILON {
            lag as f64 + f64::from(0.5 * (a - c) / denom)
        } else {
            lag as f64
        }
    } else {
        lag as f64
    };
    let energy = |x: &[f32]| x.iter().map(|v| f64::from(v * v)).sum::<f64>();
    let norm = (energy(probe) * energy(&recording[lag..lag + probe.len()])).sqrt() * n as f64;
    let confidence = if norm > 0.0 {
        (f64::from(peak) / norm).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    Some(DelayEstimate {
        samples: refined,
        confidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn delayed_chirp_is_found_within_a_millisecond() {
        let sr = 44_100;
        let probe = chirp(sr);
        for delay_ms in [0.0, 3.7, 48.0, 131.4, 287.0] {
            let delay = delay_ms / 1000.0 * f64::from(sr);
            let whole = delay.floor() as usize;
            // Fractional delay by linear interpolation, attenuated, with noise and a room echo.
            let frac = (delay - whole as f64) as f32;
            let mut rec = vec![0.0f32; sr as usize * 2];
            for (i, s) in probe.iter().enumerate() {
                rec[whole + i] += 0.3 * s * (1.0 - frac);
                rec[whole + i + 1] += 0.3 * s * frac;
                rec[whole + i + 900] += 0.08 * s;
            }
            let mut seed = 12345u32;
            for v in rec.iter_mut() {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                *v += 0.02 * ((seed >> 16) as f32 / 32768.0 - 1.0);
            }
            let est = estimate_delay(&rec, &probe, sr as usize).unwrap();
            let error_ms = (est.samples - delay) / f64::from(sr) * 1000.0;
            assert!(
                error_ms.abs() < 1.0,
                "delay {delay_ms} ms: off by {error_ms} ms"
            );
            assert!(est.confidence > 0.5, "confidence {}", est.confidence);
        }
    }

    #[test]
    fn calibration_reports_the_delay_after_the_cue() {
        let sr = 48_000;
        let probe = chirp(sr);
        // The chirp started playing 0.25 s into the capture and was heard 42 ms later.
        let at = 0.25 + 0.042;
        let mut capture = vec![0.0f32; sr as usize * 2];
        let start = (at * f64::from(sr)) as usize;
        for (i, s) in probe.iter().enumerate() {
            capture[start + i] += 0.2 * s;
        }
        let (ms, confidence) = measure_latency(&capture, sr, 0.25).unwrap();
        assert!((ms - 42.0).abs() < 1.0, "{ms} ms");
        assert!(confidence > 0.9);
        assert_eq!(
            measure_latency(&vec![0.0; sr as usize], sr, 0.1),
            Err(CalibrationError::NotHeard)
        );
    }

    #[test]
    fn count_in_is_one_bar_at_the_song_tempo() {
        assert_eq!(
            count_in(120.0, 48_000),
            PlayerCommand::CountIn {
                beats: 4,
                beat_frames: 24_000
            }
        );
        assert_eq!(
            count_in(f64::NAN, 48_000),
            PlayerCommand::CountIn {
                beats: 4,
                beat_frames: 24_000
            }
        );
        assert_eq!(
            count_in(1000.0, 48_000),
            PlayerCommand::CountIn {
                beats: 4,
                beat_frames: 12_000
            }
        );
    }

    #[test]
    fn silence_has_no_confidence() {
        let probe = chirp(16_000);
        let est = estimate_delay(&vec![0.0; 32_000], &probe, 8000).unwrap();
        assert_eq!(est.confidence, 0.0);
        assert!(estimate_delay(&probe[..100], &probe, 10).is_none());
    }

    #[test]
    fn click_accents_the_downbeat_and_rests_between_beats() {
        let sr = 48_000;
        let beat = sr / 2;
        let peak = |from: u32| {
            (from..from + 3000)
                .map(|i| click(i, beat, sr).abs())
                .fold(0.0, f32::max)
        };
        assert!(peak(0) > 0.3 && peak(beat) > 0.3);
        assert_eq!(click(beat / 2, beat, sr), 0.0);
        // Downbeat and later beats use different pitches.
        assert_ne!(click(10, beat, sr), click(beat + 10, beat, sr));
    }
}
