//! Short-time Fourier transform with the same framing as `torch.stft(center=True,
//! pad_mode="reflect")` and a Hann window, so model front-ends can be computed on the host.

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub type Spectrum = Vec<Complex32>;

pub struct Stft {
    n_fft: usize,
    hop: usize,
    window: Vec<f32>,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
}

impl Stft {
    pub fn new(n_fft: usize, hop: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        Self {
            n_fft,
            hop,
            window: hann(n_fft),
            forward: planner.plan_fft_forward(n_fft),
            inverse: planner.plan_fft_inverse(n_fft),
        }
    }

    pub fn n_fft(&self) -> usize {
        self.n_fft
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Number of bins per frame (`n_fft / 2 + 1`).
    pub fn bins(&self) -> usize {
        self.n_fft / 2 + 1
    }

    /// Frequency in Hz of bin `k`.
    pub fn bin_hz(&self, k: usize, sample_rate: u32) -> f32 {
        k as f32 * sample_rate as f32 / self.n_fft as f32
    }

    /// Centered, reflect-padded, unnormalized STFT. Returns `1 + len / hop` frames.
    pub fn forward(&self, signal: &[f32]) -> Vec<Spectrum> {
        let half = self.n_fft / 2;
        let padded = reflect_pad(signal, half, half);
        let frames = 1 + signal.len() / self.hop;
        let mut input = self.forward.make_input_vec();
        let mut scratch = self.forward.make_scratch_vec();
        (0..frames)
            .map(|f| {
                let start = f * self.hop;
                for (i, v) in input.iter_mut().enumerate() {
                    *v = padded[start + i] * self.window[i];
                }
                let mut out = self.forward.make_output_vec();
                self.forward
                    .process_with_scratch(&mut input, &mut out, &mut scratch)
                    .expect("buffer sizes come from the plan");
                out
            })
            .collect()
    }

    /// Inverse of [`Stft::forward`] (windowed overlap-add, `torch.istft(center=True)`).
    pub fn inverse(&self, frames: &[Spectrum], length: usize) -> Vec<f32> {
        let half = self.n_fft / 2;
        let total = self.n_fft + self.hop * frames.len().saturating_sub(1);
        let mut out = vec![0.0f32; total];
        let mut norm = vec![0.0f32; total];
        let mut spectrum = self.inverse.make_input_vec();
        let mut time = self.inverse.make_output_vec();
        let mut scratch = self.inverse.make_scratch_vec();
        let scale = 1.0 / self.n_fft as f32;
        for (f, frame) in frames.iter().enumerate() {
            spectrum.copy_from_slice(frame);
            // A real signal's DC and Nyquist bins are real; realfft rejects anything else.
            spectrum[0].im = 0.0;
            spectrum[half].im = 0.0;
            self.inverse
                .process_with_scratch(&mut spectrum, &mut time, &mut scratch)
                .expect("buffer sizes come from the plan");
            let start = f * self.hop;
            for i in 0..self.n_fft {
                out[start + i] += time[i] * scale * self.window[i];
                norm[start + i] += self.window[i] * self.window[i];
            }
        }
        (0..length)
            .map(|i| {
                let j = i + half;
                match (out.get(j), norm.get(j)) {
                    (Some(v), Some(n)) if *n > 1e-8 => v / n,
                    _ => 0.0,
                }
            })
            .collect()
    }
}

/// Periodic Hann window (matches `torch.hann_window`).
fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
        .collect()
}

/// Pads with mirrored samples, excluding the edge sample (numpy/torch `reflect`).
pub fn reflect_pad(signal: &[f32], left: usize, right: usize) -> Vec<f32> {
    let n = signal.len() as isize;
    let index = |i: isize| -> f32 {
        if n == 1 {
            return signal[0];
        }
        let period = 2 * (n - 1);
        let m = i.rem_euclid(period);
        signal[(if m < n { m } else { period - m }) as usize]
    };
    if n == 0 {
        return vec![0.0; left + right];
    }
    (-(left as isize)..n + right as isize).map(index).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_reconstructs_signal() {
        let stft = Stft::new(1024, 256);
        let signal: Vec<f32> = (0..10_000)
            .map(|i| (i as f32 * 0.01).sin() + 0.3 * (i as f32 * 0.137).cos())
            .collect();
        let frames = stft.forward(&signal);
        assert_eq!(frames.len(), 1 + signal.len() / 256);
        let back = stft.inverse(&frames, signal.len());
        let max_err = signal
            .iter()
            .zip(&back)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_err < 1e-4, "max error {max_err}");
    }

    #[test]
    fn sine_peaks_at_expected_bin() {
        let sr = 44_100;
        let stft = Stft::new(4096, 1024);
        let signal: Vec<f32> = (0..sr)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / sr as f32).sin())
            .collect();
        let frame = &stft.forward(&signal)[10];
        let peak = (0..frame.len())
            .max_by(|a, b| frame[*a].norm().total_cmp(&frame[*b].norm()))
            .unwrap();
        assert!((stft.bin_hz(peak, sr as u32) - 1000.0).abs() < 11.0);
    }

    #[test]
    fn reflect_pad_matches_numpy() {
        assert_eq!(
            reflect_pad(&[1.0, 2.0, 3.0], 2, 2),
            vec![3.0, 2.0, 1.0, 2.0, 3.0, 2.0, 1.0]
        );
    }
}
