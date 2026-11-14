//! Monophonic pitch estimation (YIN, de Cheveigné & Kawahara 2002).
//!
//! The difference function is computed through an FFT autocorrelation, so a frame costs
//! O(W log W) instead of O(W · τmax).

use crate::note::hz_to_midi;
use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PitchRange {
    pub min_hz: f32,
    pub max_hz: f32,
}

/// Singing voice range used everywhere in the product (AGENTS.md §6.3).
pub const VOICE_RANGE: PitchRange = PitchRange {
    min_hz: 65.0,
    max_hz: 1100.0,
};

/// CMND threshold of YIN's "absolute threshold" step.
const YIN_THRESHOLD: f32 = 0.15;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub hz: f32,
    /// CMND value at the chosen lag: 0 = perfectly periodic, ~1 = noise.
    pub aperiodicity: f32,
}

/// A reusable YIN estimator for frames of one fixed length.
pub struct Yin {
    window: usize,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    time: Vec<f32>,
    spectrum: Vec<Complex32>,
    scratch: Vec<Complex32>,
    diff: Vec<f32>,
}

impl Yin {
    pub fn new(window: usize) -> Self {
        let n = (2 * window).next_power_of_two();
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(n);
        let inverse = planner.plan_fft_inverse(n);
        let scratch_len = forward.get_scratch_len().max(inverse.get_scratch_len());
        Self {
            window,
            time: forward.make_input_vec(),
            spectrum: forward.make_output_vec(),
            scratch: vec![Complex32::default(); scratch_len],
            diff: vec![0.0; window],
            forward,
            inverse,
        }
    }

    pub fn window(&self) -> usize {
        self.window
    }

    /// Estimates the fundamental of `frame` (length must equal the window).
    /// Returns `None` when the range admits no lag for this window.
    pub fn estimate(
        &mut self,
        frame: &[f32],
        sample_rate: u32,
        range: PitchRange,
    ) -> Option<Estimate> {
        assert_eq!(
            frame.len(),
            self.window,
            "frame length must equal the YIN window"
        );
        let w = self.window;
        let sr = sample_rate as f32;
        let tau_min = ((sr / range.max_hz) as usize).max(1);
        let tau_max = ((sr / range.min_hz) as usize).min(w - 1);
        if tau_min >= tau_max {
            return None;
        }

        let mean = frame.iter().sum::<f32>() / w as f32;
        self.time.iter_mut().for_each(|v| *v = 0.0);
        for (t, x) in self.time.iter_mut().zip(frame) {
            *t = x - mean;
        }
        let x: Vec<f32> = self.time[..w].to_vec();

        // Linear autocorrelation r(τ) = IFFT(|FFT(x)|²) with zero padding to >= 2W.
        self.forward
            .process_with_scratch(&mut self.time, &mut self.spectrum, &mut self.scratch)
            .ok()?;
        for c in self.spectrum.iter_mut() {
            *c = Complex32::new(c.norm_sqr(), 0.0);
        }
        self.inverse
            .process_with_scratch(&mut self.spectrum, &mut self.time, &mut self.scratch)
            .ok()?;
        let scale = 1.0 / self.time.len() as f32;

        // d(τ) = Σ_{j<W-τ} x_j² + Σ_{j>=τ} x_j² − 2 r(τ)
        let mut head = vec![0.0f32; w + 1];
        for j in 0..w {
            head[j + 1] = head[j] + x[j] * x[j];
        }
        let total = head[w];
        for tau in 0..=tau_max {
            let a = head[w - tau];
            let b = total - head[tau];
            self.diff[tau] = (a + b - 2.0 * self.time[tau] * scale).max(0.0);
        }

        // Cumulative mean normalized difference.
        let mut cmnd = vec![1.0f32; tau_max + 1];
        let mut running = 0.0f32;
        for (tau, (c, d)) in cmnd.iter_mut().zip(&self.diff).enumerate().skip(1) {
            running += d;
            *c = d * tau as f32 / (running + 1e-12);
        }

        // First dip below the threshold (descend to its local minimum), else the global minimum.
        let tau = match (tau_min..=tau_max).find(|&t| cmnd[t] < YIN_THRESHOLD) {
            Some(mut t) => {
                while t < tau_max && cmnd[t + 1] < cmnd[t] {
                    t += 1;
                }
                t
            }
            None => (tau_min..=tau_max).min_by(|a, b| cmnd[*a].total_cmp(&cmnd[*b]))?,
        };

        // Parabolic refinement of the minimum.
        let refined = if tau > tau_min && tau < tau_max {
            let (a, b, c) = (cmnd[tau - 1], cmnd[tau], cmnd[tau + 1]);
            let denom = a - 2.0 * b + c;
            if denom.abs() > 1e-12 {
                tau as f32 + 0.5 * (a - c) / denom
            } else {
                tau as f32
            }
        } else {
            tau as f32
        };
        Some(Estimate {
            hz: sr / refined,
            aperiodicity: cmnd[tau],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
/// Pitch is stored as fractional MIDI (A4 = 69) so consumers never convert units.
pub enum PitchFrame {
    Voiced { midi: f32 },
    Unvoiced,
}

/// A pitch contour sampled every `hop_seconds`; frame `i` is centered at `i * hop_seconds`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PitchTrack {
    pub hop_seconds: f64,
    pub frames: Vec<PitchFrame>,
}

impl PitchTrack {
    pub fn time(&self, frame: usize) -> f64 {
        frame as f64 * self.hop_seconds
    }

    pub fn frame_at(&self, seconds: f64) -> Option<&PitchFrame> {
        if seconds < 0.0 {
            return None;
        }
        self.frames
            .get((seconds / self.hop_seconds).round() as usize)
    }
}

/// Analysis settings for [`track`]: ~46 ms window, 10 ms hop (AGENTS.md §4.4, §6.3).
pub const TRACK_WINDOW_SECONDS: f64 = 0.046;
pub const TRACK_HOP_SECONDS: f64 = 0.010;
/// Frames with YIN aperiodicity above this are unvoiced.
const VOICING_THRESHOLD: f32 = 0.25;
/// Frames quieter than this RMS (about -50 dBFS) are unvoiced.
const SILENCE_RMS: f32 = 0.003;

/// Pitch contour of a mono signal using [`VOICE_RANGE`].
pub fn track(signal: &[f32], sample_rate: u32) -> PitchTrack {
    let window = (TRACK_WINDOW_SECONDS * sample_rate as f64) as usize;
    let hop = (TRACK_HOP_SECONDS * sample_rate as f64) as usize;
    let mut yin = Yin::new(window);
    let mut frame = vec![0.0f32; window];
    let count = signal.len() / hop + 1;
    let frames = (0..count)
        .map(|i| {
            // Center frame i on sample i * hop, zero-padded at the edges.
            let start = (i * hop) as isize - (window / 2) as isize;
            for (k, v) in frame.iter_mut().enumerate() {
                let idx = start + k as isize;
                *v = if idx >= 0 && (idx as usize) < signal.len() {
                    signal[idx as usize]
                } else {
                    0.0
                };
            }
            let rms = (frame.iter().map(|v| v * v).sum::<f32>() / window as f32).sqrt();
            match yin.estimate(&frame, sample_rate, VOICE_RANGE) {
                Some(e) if rms >= SILENCE_RMS && e.aperiodicity <= VOICING_THRESHOLD => {
                    PitchFrame::Voiced {
                        midi: hz_to_midi(e.hz),
                    }
                }
                _ => PitchFrame::Unvoiced,
            }
        })
        .collect();
    PitchTrack {
        hop_seconds: hop as f64 / sample_rate as f64,
        frames,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::note::{cents, midi_to_hz};
    use std::f32::consts::PI;

    pub fn sine(hz: f32, seconds: f32, sr: u32) -> Vec<f32> {
        (0..(seconds * sr as f32) as usize)
            .map(|i| 0.5 * (2.0 * PI * hz * i as f32 / sr as f32).sin())
            .collect()
    }

    /// A voice-like tone: fundamental plus decaying harmonics.
    pub fn harmonic(hz: f32, seconds: f32, sr: u32) -> Vec<f32> {
        (0..(seconds * sr as f32) as usize)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (1..=6)
                    .map(|h| 0.4 / h as f32 * (2.0 * PI * hz * h as f32 * t).sin())
                    .sum()
            })
            .collect()
    }

    #[test]
    fn yin_finds_sines_within_two_cents() {
        let sr = 44_100;
        let mut yin = Yin::new(2048);
        for hz in [82.41, 110.0, 220.0, 329.63, 440.0, 880.0] {
            let x = sine(hz, 0.1, sr);
            let e = yin.estimate(&x[..2048], sr, VOICE_RANGE).unwrap();
            assert!(cents(e.hz, hz).abs() < 2.0, "{hz}: got {}", e.hz);
            assert!(e.aperiodicity < 0.05);
        }
    }

    #[test]
    fn yin_avoids_octave_errors_on_harmonic_tones() {
        let sr = 44_100;
        let mut yin = Yin::new(2048);
        for hz in [98.0, 196.0, 392.0] {
            let x = harmonic(hz, 0.1, sr);
            let e = yin.estimate(&x[..2048], sr, VOICE_RANGE).unwrap();
            assert!(cents(e.hz, hz).abs() < 5.0, "{hz}: got {}", e.hz);
        }
    }

    #[test]
    fn track_follows_a_chirp_and_marks_silence_unvoiced() {
        let sr = 44_100;
        // 1 s exponential chirp 200 -> 400 Hz, then 0.5 s of silence.
        let mut x: Vec<f32> = (0..sr as usize)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let phase = 2.0 * PI * 200.0 * (2f32.powf(t) - 1.0) / 2f32.ln();
                0.5 * phase.sin()
            })
            .collect();
        x.extend(std::iter::repeat_n(0.0, sr as usize / 2));
        let track = track(&x, sr);
        assert!((track.hop_seconds - 0.01).abs() < 1e-3);
        for i in (10..90).step_by(10) {
            let t = track.time(i) as f32;
            let expected = 200.0 * 2f32.powf(t);
            match track.frames[i] {
                PitchFrame::Voiced { midi } => {
                    let hz = midi_to_hz(midi);
                    assert!(
                        cents(hz, expected).abs() < 10.0,
                        "t={t}: {hz} vs {expected}"
                    )
                }
                PitchFrame::Unvoiced => panic!("frame {i} unvoiced"),
            }
        }
        assert!(track.frames[120..]
            .iter()
            .all(|f| *f == PitchFrame::Unvoiced));
    }
}
