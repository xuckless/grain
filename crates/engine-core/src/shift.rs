//! Offline, high-quality pitch shifting with formant preservation (AGENTS.md §7 step 4).
//!
//! A frequency-domain phase vocoder on [`crate::stft::Stft`]: each frame's magnitude is divided
//! by its spectral envelope (cepstral liftering), the whitened partials move to `k · ratio` with
//! their phase advance scaled to match, and the envelope (optionally formant-shifted) is put
//! back. The voice keeps its vowels and character while its pitch moves. Frames with no
//! correction pass through untouched, so consonants and breaths are bit-for-bit unchanged.
//!
//! Pure Rust on realfft: builds for wasm32, so desktop and web share it.

use crate::stft::{Complex, Stft};
use realfft::{ComplexToReal, RealFftPlanner};
use std::f32::consts::PI;
use std::sync::Arc;

/// Analysis window (~46 ms at 44.1 kHz) and 4× overlap.
const WINDOW_SECONDS: f64 = 0.046;
const OVERLAP: usize = 4;
/// Pitch range searched in the cepstrum for the frame's period (the voice range).
const PERIOD_MIN_HZ: f32 = 65.0;
const PERIOD_MAX_HZ: f32 = 1100.0;

/// The STFT used by [`shift`] at `sample_rate`.
fn stft_for(sample_rate: u32) -> Stft {
    let n_fft = ((WINDOW_SECONDS * f64::from(sample_rate)) as usize).next_power_of_two();
    Stft::new(n_fft, n_fft / OVERLAP)
}

/// Shifts `signal` by `correction[i]` semitones around time `i · hop_seconds` (linearly
/// interpolated; 0 outside the curve), and its formants by `formant_semitones` wherever a
/// frame is corrected. Returns as many samples as it was given.
pub fn shift(
    signal: &[f32],
    sample_rate: u32,
    correction: &[f32],
    hop_seconds: f64,
    formant_semitones: f32,
) -> Vec<f32> {
    shift_with(signal, sample_rate, hop_seconds, |x| {
        (at(correction, x), formant_semitones)
    })
}

/// [`shift`] with the formant shift as a curve too (per-note formants, AGENTS.md §7.1):
/// `formant[i]` semitones around time `i · hop_seconds`, 0 outside the curve.
pub fn shift_curves(
    signal: &[f32],
    sample_rate: u32,
    correction: &[f32],
    formant: &[f32],
    hop_seconds: f64,
) -> Vec<f32> {
    shift_with(signal, sample_rate, hop_seconds, |x| {
        (at(correction, x), at(formant, x))
    })
}

/// The shifter, with `curves(x)` giving (pitch, formant) semitones at fractional curve index `x`.
fn shift_with(
    signal: &[f32],
    sample_rate: u32,
    hop_seconds: f64,
    curves: impl Fn(f64) -> (f32, f32),
) -> Vec<f32> {
    let stft = stft_for(sample_rate);
    let mut vocoder = Vocoder::new(&stft, sample_rate);
    let frame_seconds = stft.hop() as f64 / f64::from(sample_rate);
    stft.map(signal, |f, spectrum| {
        let (semitones, formant) = curves(f as f64 * frame_seconds / hop_seconds);
        vocoder.frame(
            spectrum,
            2f32.powf(semitones / 12.0),
            2f32.powf(formant / 12.0),
        );
    })
}

/// `curve` at fractional index `x`, linearly interpolated, 0 outside.
fn at(curve: &[f32], x: f64) -> f32 {
    if x < 0.0 {
        return 0.0;
    }
    let i = x.floor() as usize;
    let t = (x - i as f64) as f32;
    let a = curve.get(i).copied().unwrap_or(0.0);
    let b = curve.get(i + 1).copied().unwrap_or(0.0);
    a + t * (b - a)
}

/// Wraps a phase into (−π, π].
fn principal(phase: f32) -> f32 {
    phase - 2.0 * PI * ((phase + PI) / (2.0 * PI)).floor()
}

/// Phase-vocoder state carried from frame to frame.
struct Vocoder {
    hop: usize,
    n_fft: usize,
    /// Phase of each input bin this frame (raw) and the previous frame.
    phase: Vec<f32>,
    last_in: Vec<f32>,
    /// Zero-phase-referenced phase of each output bin in the previous frame (see `centred`).
    last_out: Vec<f32>,
    magnitude: Vec<f32>,
    /// Phase advance per hop of each input bin's partial.
    advance: Vec<f32>,
    /// The spectral peaks (input bins) this frame; capacity `bins`, so pushing never allocates.
    peaks: Vec<usize>,
    /// The peak each input bin belongs to.
    peak_of: Vec<usize>,
    /// Output phase of each peak's partial, indexed by its input bin.
    peak_phase: Vec<f32>,
    envelope: Envelope,
    log_envelope: Vec<f32>,
}

impl Vocoder {
    fn new(stft: &Stft, sample_rate: u32) -> Self {
        let bins = stft.bins();
        Self {
            hop: stft.hop(),
            n_fft: stft.n_fft(),
            phase: vec![0.0; bins],
            last_in: vec![0.0; bins],
            last_out: vec![0.0; bins],
            magnitude: vec![0.0; bins],
            advance: vec![0.0; bins],
            peaks: Vec::with_capacity(bins),
            peak_of: vec![0; bins],
            peak_phase: vec![0.0; bins],
            envelope: Envelope::new(stft.n_fft(), sample_rate),
            log_envelope: vec![0.0; bins],
        }
    }

    /// Expected phase advance per hop of bin `k`'s centre frequency.
    fn bin_advance(&self, k: usize) -> f32 {
        2.0 * PI * k as f32 * self.hop as f32 / self.n_fft as f32
    }

    /// Shifts one frame in place by `ratio`, formants by `formant`.
    ///
    /// Peak-locked (Laroche & Dolson 1999): each spectral peak's partial moves to bin
    /// `peak · ratio` with its frequency scaled; the bins around it keep their phase relative
    /// to the peak, so a partial stays one coherent sinusoid instead of beating with itself.
    fn frame(&mut self, spectrum: &mut [Complex], ratio: f32, formant: f32) {
        let bins = spectrum.len();
        for (k, c) in spectrum.iter().enumerate() {
            self.phase[k] = c.arg();
            self.magnitude[k] = c.norm();
        }
        let unchanged = (ratio - 1.0).abs() < 1e-5 && (formant - 1.0).abs() < 1e-5;
        if unchanged {
            // Pass through, keeping both phase memories in step with the input.
            self.last_in.copy_from_slice(&self.phase);
            for k in 0..bins {
                self.last_out[k] = centred(self.phase[k], k);
            }
            return;
        }
        for k in 0..bins {
            let expected = self.bin_advance(k);
            self.advance[k] = expected + principal(self.phase[k] - self.last_in[k] - expected);
        }
        self.last_in.copy_from_slice(&self.phase);
        self.envelope
            .log_envelope(&self.magnitude, &mut self.log_envelope);

        // Peaks, and each bin's nearest peak (regions split halfway between peaks).
        self.peaks.clear();
        let m = &self.magnitude;
        for k in 2..bins - 2 {
            if m[k] > m[k - 1] && m[k] >= m[k + 1] && m[k] > m[k - 2] && m[k] >= m[k + 2] {
                self.peaks.push(k);
            }
        }
        if self.peaks.is_empty() {
            spectrum.fill(Complex::default());
            self.last_out.fill(0.0);
            return;
        }
        let mut region = 0;
        for k in 0..bins {
            while region + 1 < self.peaks.len()
                && k * 2 > self.peaks[region] + self.peaks[region + 1]
            {
                region += 1;
            }
            self.peak_of[k] = self.peaks[region];
        }
        for &p in &self.peaks {
            let j = ((p as f32 * ratio).round() as usize).min(bins - 1);
            self.peak_phase[p] = self.last_out[j] + self.advance[p] * ratio;
        }

        // Each output bin reads the input bin it came from: whitened, locked to its peak, then
        // given the (optionally formant-shifted) envelope at its new place.
        for (j, c) in spectrum.iter_mut().enumerate() {
            let k = (j as f32 / ratio).round() as usize;
            if k >= bins {
                *c = Complex::default();
                self.last_out[j] = 0.0;
                continue;
            }
            let p = self.peak_of[k];
            let locked = self.peak_phase[p] + centred(self.phase[k], k) - centred(self.phase[p], p);
            self.last_out[j] = principal(locked);
            let phase = locked - PI * (j % 2) as f32;
            let white = self.magnitude[k] / self.log_envelope[k].exp();
            let level = white * envelope_at(&self.log_envelope, j as f32 / formant).exp();
            *c = Complex::from_polar(level.min(f32::MAX), phase);
        }
    }
}

/// Bin `k`'s phase as if the analysis window were centred on time 0 (a circular shift by half
/// a frame, `+πk`). In that frame a steady partial's main-lobe bins share one phase, so bins
/// read from the same or neighbouring source bins stay coherent when the lobe is stretched.
fn centred(phase: f32, k: usize) -> f32 {
    phase + PI * (k % 2) as f32
}

/// `values` at fractional bin `x`, linearly interpolated (clamped to its ends).
fn envelope_at(values: &[f32], x: f32) -> f32 {
    let last = values.len() - 1;
    let x = x.clamp(0.0, last as f32);
    let i = (x.floor() as usize).min(last - 1);
    let t = x - i as f32;
    values[i] + t * (values[i + 1] - values[i])
}

/// Spectral envelope of a voiced frame: the curve through its harmonic peaks. The pitch period
/// comes from the cepstrum; the envelope is the log magnitude's running maximum over one
/// harmonic spacing (so every bin sees a harmonic), smoothed over the same width. Unlike a
/// liftered cepstrum, it passes through the peaks instead of averaging them with the valleys.
pub(crate) struct Envelope {
    inverse: Arc<dyn ComplexToReal<f32>>,
    spectrum: Vec<Complex>,
    cepstrum: Vec<f32>,
    scratch: Vec<Complex>,
    /// Quefrency range (samples) of the voice's pitch periods.
    periods: std::ops::RangeInclusive<usize>,
    /// Running maximum of the log magnitude.
    peaks: Vec<f32>,
}

impl Envelope {
    pub(crate) fn new(n_fft: usize, sample_rate: u32) -> Self {
        let inverse = RealFftPlanner::<f32>::new().plan_fft_inverse(n_fft);
        Self {
            spectrum: inverse.make_input_vec(),
            cepstrum: inverse.make_output_vec(),
            scratch: inverse.make_scratch_vec(),
            periods: (sample_rate as f32 / PERIOD_MAX_HZ) as usize
                ..=((sample_rate as f32 / PERIOD_MIN_HZ) as usize).min(n_fft / 2 - 1),
            peaks: vec![0.0; n_fft / 2 + 1],
            inverse,
        }
    }

    /// Natural-log envelope of a magnitude spectrum (`n_fft / 2 + 1` bins) into `out`.
    pub(crate) fn log_envelope(&mut self, magnitude: &[f32], out: &mut [f32]) {
        let n = self.cepstrum.len();
        for (s, m) in self.spectrum.iter_mut().zip(magnitude) {
            *s = Complex::new((m + 1e-9).ln(), 0.0);
        }
        self.inverse
            .process_with_scratch(&mut self.spectrum, &mut self.cepstrum, &mut self.scratch)
            .expect("buffer sizes come from the plan");
        // The pitch period is the cepstrum's peak in the voice range.
        let period = self
            .periods
            .clone()
            .max_by(|a, b| self.cepstrum[*a].total_cmp(&self.cepstrum[*b]))
            .unwrap_or(*self.periods.start());
        // Half a harmonic spacing either side.
        let half = ((n as f32 / period as f32 / 2.0).ceil() as usize).max(1);
        let bins = magnitude.len();
        for k in 0..bins {
            let window = &magnitude[k.saturating_sub(half)..(k + half + 1).min(bins)];
            self.peaks[k] = (window.iter().cloned().fold(0.0f32, f32::max) + 1e-9).ln();
        }
        for (k, o) in out.iter_mut().enumerate() {
            let window = &self.peaks[k.saturating_sub(half)..(k + half + 1).min(bins)];
            *o = window.iter().sum::<f32>() / window.len() as f32;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::note::{cents, midi_to_hz};
    use crate::pitch::{self, PitchFrame};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 44_100;

    /// The /a/ formants: centre frequency and bandwidth (Hz).
    const FORMANTS: [(f32, f32); 3] = [(730.0, 90.0), (1090.0, 110.0), (2440.0, 170.0)];

    /// One resonator's coefficients `(gain, a1, a2)`: `y = gain·x + a1·y[−1] + a2·y[−2]`.
    fn resonator(f: f32, bw: f32, sr: u32) -> (f32, f32, f32) {
        let r = (-PI * bw / sr as f32).exp();
        (1.0 - r, 2.0 * r * (2.0 * PI * f / sr as f32).cos(), -r * r)
    }

    /// The vowel's true envelope at `hz` (dB): the resonators' magnitude response.
    pub(crate) fn formant_db(hz: f32, sr: u32) -> f32 {
        let w = 2.0 * PI * hz / sr as f32;
        FORMANTS
            .iter()
            .map(|&(f, bw)| {
                let (g, a1, a2) = resonator(f, bw, sr);
                let re = 1.0 - a1 * w.cos() - a2 * (2.0 * w).cos();
                let im = a1 * w.sin() + a2 * (2.0 * w).sin();
                20.0 * (g / (re * re + im * im).sqrt()).log10()
            })
            .sum()
    }

    /// A sung vowel at `hz`: a flat pulse train (every harmonic equally loud, so the vowel's
    /// envelope is exactly [`formant_db`]) through the three /a/ formant resonators.
    pub(crate) fn vowel(hz: f32, seconds: f32, sr: u32) -> Vec<f32> {
        let n = (seconds * sr as f32) as usize;
        let harmonics = (0.45 * sr as f32 / hz) as usize;
        let mut out: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (1..=harmonics)
                    .map(|h| (2.0 * PI * hz * h as f32 * t).cos())
                    .sum::<f32>()
            })
            .collect();
        for (f, bw) in FORMANTS {
            let (g, a1, a2) = resonator(f, bw, sr);
            let (mut y1, mut y2) = (0.0f32, 0.0f32);
            for x in out.iter_mut() {
                let y = g * *x + a1 * y1 + a2 * y2;
                y2 = y1;
                y1 = y;
                *x = y;
            }
        }
        let peak = out[n / 10..].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        out.iter().map(|v| 0.5 * v / peak).collect()
    }

    /// RMS error (dB, mean level removed) between the harmonic amplitudes of `signal` (pitch
    /// `hz`) and the vowel's true envelope, over harmonics between 200 Hz and 4 kHz. Small when
    /// the formants stayed where the vowel has them.
    pub(crate) fn formant_error(signal: &[f32], hz: f32, sr: u32) -> f32 {
        let stft = Stft::new(8192, 2048);
        let frames = stft.forward(signal);
        let usable = &frames[frames.len() / 4..3 * frames.len() / 4];
        let mean: Vec<f32> = (0..stft.bins())
            .map(|k| usable.iter().map(|f| f[k].norm()).sum::<f32>() / usable.len() as f32)
            .collect();
        let bin_hz = sr as f32 / stft.n_fft() as f32;
        let pairs: Vec<(f32, f32)> = (1..)
            .map(|h| h as f32 * hz)
            .skip_while(|f| *f < 200.0)
            .take_while(|f| *f < 4000.0)
            .map(|f| {
                let k = (f / bin_hz).round() as usize;
                let peak = mean[k - 2..=k + 2].iter().cloned().fold(0.0f32, f32::max);
                (20.0 * (peak + 1e-9).log10(), formant_db(f, sr))
            })
            .collect();
        let offset = pairs.iter().map(|(m, t)| m - t).sum::<f32>() / pairs.len() as f32;
        (pairs
            .iter()
            .map(|(m, t)| (m - t - offset).powi(2))
            .sum::<f32>()
            / pairs.len() as f32)
            .sqrt()
    }

    /// Median pitch (Hz) of the steady middle of a signal.
    pub(crate) fn median_hz(signal: &[f32], sr: u32) -> f32 {
        let track = pitch::track(signal, sr);
        let n = track.frames.len();
        let mut hz: Vec<f32> = track.frames[n / 4..3 * n / 4]
            .iter()
            .filter_map(|f| match f {
                PitchFrame::Voiced { midi } => Some(midi_to_hz(*midi)),
                PitchFrame::Unvoiced => None,
            })
            .collect();
        assert!(hz.len() > n / 4, "mostly voiced");
        hz.sort_by(f32::total_cmp);
        hz[hz.len() / 2]
    }

    /// Plain resampling by `ratio`: pitch and formants move together (the "chipmunk" effect).
    pub(crate) fn resampled(input: &[f32], ratio: f32) -> Vec<f32> {
        (0..(input.len() as f32 / ratio) as usize - 1)
            .map(|i| {
                let x = i as f32 * ratio;
                let j = x as usize;
                input[j] + (x - j as f32) * (input[j + 1] - input[j])
            })
            .collect()
    }

    fn constant(semitones: f32, seconds: f32) -> Vec<f32> {
        vec![semitones; (seconds / 0.01) as usize + 2]
    }

    #[test]
    fn shifts_pitch_within_five_cents() {
        let input = vowel(220.0, 1.0, SR);
        for semitones in [1.0f32, -1.0, 3.0, -3.0, 12.0, -12.0] {
            let out = shift(&input, SR, &constant(semitones, 1.0), 0.01, 0.0);
            assert_eq!(out.len(), input.len());
            let got = median_hz(&out, SR);
            let want = 220.0 * 2f32.powf(semitones / 12.0);
            assert!(
                cents(got, want).abs() < 5.0,
                "{semitones:+} st: {got} Hz vs {want} Hz"
            );
        }
    }

    #[test]
    fn keeps_the_vowel_while_moving_the_pitch() {
        let input = vowel(196.0, 1.0, SR);
        let floor = formant_error(&input, 196.0, SR);
        let ratio = 2f32.powf(3.0 / 12.0);
        let ours = formant_error(
            &shift(&input, SR, &constant(3.0, 1.0), 0.01, 0.0),
            196.0 * ratio,
            SR,
        );
        let chipmunk = formant_error(&resampled(&input, ratio), 196.0 * ratio, SR);
        assert!(floor < 1.0, "measurement floor {floor} dB");
        assert!(ours < 3.0, "formants off by {ours} dB");
        assert!(
            ours < chipmunk / 2.0,
            "ours {ours} dB vs resampling {chipmunk} dB"
        );
    }

    #[test]
    fn round_trip_returns_the_original_voice() {
        let input = vowel(220.0, 1.0, SR);
        let up = shift(&input, SR, &constant(3.0, 1.0), 0.01, 0.0);
        let back = shift(&up, SR, &constant(-3.0, 1.0), 0.01, 0.0);
        let got = median_hz(&back, SR);
        assert!(cents(got, 220.0).abs() < 5.0, "{got} Hz");
        let error = formant_error(&back, 220.0, SR);
        // Two passes, each within ~2 dB.
        assert!(error < 3.0, "round trip changed the timbre by {error} dB");
        // Log-spectral distance of the whole spectra, frame by frame, stays small.
        let lsd = log_spectral_distance(&input, &back);
        assert!(lsd < 4.0, "log-spectral distance {lsd} dB");
    }

    #[test]
    fn uncorrected_frames_pass_through_unchanged() {
        let input = vowel(220.0, 0.5, SR);
        let out = shift(&input, SR, &constant(0.0, 0.5), 0.01, 0.0);
        let worst = input
            .iter()
            .zip(&out)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst < 1e-4, "{worst}");
    }

    #[test]
    fn formant_shift_moves_the_envelope_not_the_pitch() {
        let input = vowel(196.0, 1.0, SR);
        let out = shift(&input, SR, &constant(0.0, 1.0), 0.01, 3.0);
        assert!(cents(median_hz(&out, SR), 196.0).abs() < 5.0);
        let moved = formant_error(&out, 196.0, SR);
        assert!(moved > 3.0, "formants should move: {moved} dB");
    }

    /// Offline speed, for the record (`cargo test --release -p engine-core offline_speed`).
    #[test]
    #[cfg_attr(any(debug_assertions, target_arch = "wasm32"), ignore)]
    fn offline_speed() {
        let seconds = 20.0;
        let input = vowel(196.0, seconds, SR);
        let curve: Vec<f32> = (0..(seconds / 0.01) as usize)
            .map(|i| 0.3 * (i as f32 * 0.01).sin())
            .collect();
        let started = std::time::Instant::now();
        let out = shift(&input, SR, &curve, 0.01, 0.0);
        let factor = started.elapsed().as_secs_f32() / seconds;
        eprintln!(
            "offline shifter: {:.1}× faster than real time",
            1.0 / factor
        );
        assert_eq!(out.len(), input.len());
        assert!(factor < 0.5, "{factor}");
    }

    /// Log-spectral distance: mean over frames of the RMS dB difference of the magnitude
    /// spectra (200 Hz – 4 kHz).
    fn log_spectral_distance(a: &[f32], b: &[f32]) -> f32 {
        let stft = stft_for(SR);
        let (fa, fb) = (stft.forward(a), stft.forward(b));
        let n = fa.len().min(fb.len());
        let bins = (200.0 * stft.n_fft() as f32 / SR as f32) as usize
            ..(4000.0 * stft.n_fft() as f32 / SR as f32) as usize;
        // dB, floored 50 dB below the frame's peak (the valleys between harmonics are noise).
        let db = |frame: &[Complex], k: usize| {
            let peak = frame.iter().fold(0.0f32, |m, c| m.max(c.norm()));
            20.0 * frame[k].norm().max(peak * 0.003_16).max(1e-9).log10()
        };
        let frames = 4..n - 4;
        frames
            .clone()
            .map(|f| {
                (bins
                    .clone()
                    .map(|k| (db(&fa[f], k) - db(&fb[f], k)).powi(2))
                    .sum::<f32>()
                    / bins.len() as f32)
                    .sqrt()
            })
            .sum::<f32>()
            / frames.len() as f32
    }
}
