//! Short-time Fourier transform with the same framing as `torch.stft(center=True,
//! pad_mode="reflect")` and a Hann window, so model front-ends can be computed on the host.

use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub use realfft::num_complex::Complex32 as Complex;
pub type Spectrum = Vec<Complex>;

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
        let padded = reflect_pad(signal, self.n_fft / 2, self.n_fft / 2);
        let mut buffers = Buffers::new(self);
        (0..1 + signal.len() / self.hop)
            .map(|f| {
                let mut out = self.forward.make_output_vec();
                self.analyze(&padded, f, &mut buffers, &mut out);
                out
            })
            .collect()
    }

    /// Inverse of [`Stft::forward`] (windowed overlap-add, `torch.istft(center=True)`).
    pub fn inverse(&self, frames: &[Spectrum], length: usize) -> Vec<f32> {
        let mut ola = OverlapAdd::new(self, frames.len());
        let mut buffers = Buffers::new(self);
        for (f, frame) in frames.iter().enumerate() {
            buffers.spectrum.copy_from_slice(frame);
            self.synthesize(f, &mut buffers, &mut ola);
        }
        ola.finish(self.n_fft / 2, length)
    }

    /// Forward STFT, `edit(frame index, spectrum)` on each frame in order, then the inverse:
    /// the same result as `inverse(forward(signal) edited)` without holding every frame.
    pub fn map(&self, signal: &[f32], mut edit: impl FnMut(usize, &mut [Complex])) -> Vec<f32> {
        let padded = reflect_pad(signal, self.n_fft / 2, self.n_fft / 2);
        let frames = 1 + signal.len() / self.hop;
        let mut ola = OverlapAdd::new(self, frames);
        let mut buffers = Buffers::new(self);
        let mut spectrum = self.forward.make_output_vec();
        for f in 0..frames {
            self.analyze(&padded, f, &mut buffers, &mut spectrum);
            edit(f, &mut spectrum);
            buffers.spectrum.copy_from_slice(&spectrum);
            self.synthesize(f, &mut buffers, &mut ola);
        }
        ola.finish(self.n_fft / 2, signal.len())
    }

    /// Windowed FFT of frame `f` of the padded signal into `out`.
    fn analyze(&self, padded: &[f32], f: usize, b: &mut Buffers, out: &mut [Complex]) {
        let start = f * self.hop;
        for (i, v) in b.input.iter_mut().enumerate() {
            *v = padded[start + i] * self.window[i];
        }
        self.forward
            .process_with_scratch(&mut b.input, out, &mut b.scratch)
            .expect("buffer sizes come from the plan");
    }

    /// Inverse FFT of `b.spectrum`, windowed and added at frame `f`.
    fn synthesize(&self, f: usize, b: &mut Buffers, ola: &mut OverlapAdd) {
        let half = self.n_fft / 2;
        // A real signal's DC and Nyquist bins are real; realfft rejects anything else.
        b.spectrum[0].im = 0.0;
        b.spectrum[half].im = 0.0;
        self.inverse
            .process_with_scratch(&mut b.spectrum, &mut b.time, &mut b.scratch)
            .expect("buffer sizes come from the plan");
        let scale = 1.0 / self.n_fft as f32;
        let start = f * self.hop;
        for i in 0..self.n_fft {
            ola.out[start + i] += b.time[i] * scale * self.window[i];
            ola.norm[start + i] += self.window[i] * self.window[i];
        }
    }
}

/// Per-transform FFT buffers.
struct Buffers {
    input: Vec<f32>,
    spectrum: Vec<Complex>,
    time: Vec<f32>,
    scratch: Vec<Complex>,
}

impl Buffers {
    fn new(stft: &Stft) -> Self {
        let scratch = stft
            .forward
            .get_scratch_len()
            .max(stft.inverse.get_scratch_len());
        Self {
            input: stft.forward.make_input_vec(),
            spectrum: stft.inverse.make_input_vec(),
            time: stft.inverse.make_output_vec(),
            scratch: vec![Complex::default(); scratch],
        }
    }
}

/// The overlap-add accumulator of an inverse STFT.
struct OverlapAdd {
    out: Vec<f32>,
    norm: Vec<f32>,
}

impl OverlapAdd {
    fn new(stft: &Stft, frames: usize) -> Self {
        let total = stft.n_fft + stft.hop * frames.saturating_sub(1);
        Self {
            out: vec![0.0; total],
            norm: vec![0.0; total],
        }
    }

    /// Window-normalized output, `length` samples from `half` (undoing the centering pad).
    fn finish(self, half: usize, length: usize) -> Vec<f32> {
        (0..length)
            .map(|i| {
                let j = i + half;
                match (self.out.get(j), self.norm.get(j)) {
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
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

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
    fn map_matches_forward_then_inverse() {
        let stft = Stft::new(512, 128);
        let signal: Vec<f32> = (0..5_000).map(|i| (i as f32 * 0.03).sin()).collect();
        // Identity edit reconstructs; a scaling edit matches the batch path.
        let same = stft.map(&signal, |_, _| {});
        let mut frames = stft.forward(&signal);
        frames
            .iter_mut()
            .enumerate()
            .for_each(|(f, s)| s.iter_mut().for_each(|c| *c *= 1.0 + f as f32 * 0.001));
        let batch = stft.inverse(&frames, signal.len());
        let mapped = stft.map(&signal, |f, s| {
            s.iter_mut().for_each(|c| *c *= 1.0 + f as f32 * 0.001)
        });
        for i in 0..signal.len() {
            assert!((same[i] - signal[i]).abs() < 1e-4);
            assert!((mapped[i] - batch[i]).abs() < 1e-5);
        }
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
