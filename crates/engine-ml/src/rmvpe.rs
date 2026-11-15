//! RMVPE vocal pitch (AGENTS.md §4.4, §5), host side: the log-mel front end and the salience
//! decoding of the RVC project's `rmvpe.py`, around the published `rmvpe.onnx`:
//!
//! ```text
//! input  input  [1, 128, T]  log-mel, T padded to a multiple of 32 frames
//! output output [1, T, 360]  pitch salience in 20-cent bins
//! ```

use engine_core::note::hz_to_midi;
use engine_core::pitch::{PitchFrame, PitchTrack};
use engine_core::stft::Stft;

/// The model's input rate; 160-sample hops make 10 ms frames.
pub const SAMPLE_RATE: u32 = 16_000;
const N_FFT: usize = 1024;
const HOP: usize = 160;
pub const N_MELS: usize = 128;
const FMIN: f32 = 30.0;
const FMAX: f32 = 8000.0;
const LOG_FLOOR: f32 = 1e-5;
pub const BINS: usize = 360;
/// Frames whose peak salience is at or below this are unvoiced (`rmvpe.py`, `thred=0.03`).
const VOICING_THRESHOLD: f32 = 0.03;
/// Cents (relative to 10 Hz) of salience bin 0.
const FIRST_BIN_CENTS: f32 = 1_997.379_4;
/// Frame counts the network accepts are multiples of this.
const FRAME_MULTIPLE: usize = 32;

/// One analysis's model input.
pub struct RmvpeInput {
    /// `[N_MELS, padded]`, row-major.
    pub mel: Vec<f32>,
    /// Real frames (one per 10 ms of audio, plus one).
    pub frames: usize,
    /// Frames after padding to a multiple of 32.
    pub padded: usize,
}

/// Log-mel spectrogram of mono audio at [`SAMPLE_RATE`] (`rmvpe.py` `MelSpectrogram`, with
/// the trailing zero padding of `mel2hidden`).
pub fn features(signal: &[f32]) -> RmvpeInput {
    let spectra = Stft::new(N_FFT, HOP).forward(signal);
    let filters = mel_filters();
    let frames = spectra.len();
    let padded = frames.div_ceil(FRAME_MULTIPLE).max(1) * FRAME_MULTIPLE;
    let mut mel = vec![0.0f32; N_MELS * padded];
    for (t, frame) in spectra.iter().enumerate() {
        let magnitude: Vec<f32> = frame.iter().map(|z| z.norm()).collect();
        for (m, filter) in filters.iter().enumerate() {
            let energy: f32 = filter.iter().zip(&magnitude).map(|(w, x)| w * x).sum();
            mel[m * padded + t] = energy.max(LOG_FLOOR).ln();
        }
    }
    RmvpeInput {
        mel,
        frames,
        padded,
    }
}

/// Pitch track from the model's salience (`[padded, BINS]`, row-major): a salience-weighted
/// average of the 9 bins around each frame's peak (`to_local_average_cents`).
pub fn decode(salience: &[f32], frames: usize) -> PitchTrack {
    let frames = (0..frames)
        .map(|t| {
            let row = &salience[t * BINS..(t + 1) * BINS];
            let (peak, max) =
                row.iter().enumerate().fold(
                    (0, f32::MIN),
                    |best, (i, &v)| if v > best.1 { (i, v) } else { best },
                );
            if max <= VOICING_THRESHOLD {
                return PitchFrame::Unvoiced;
            }
            let window = peak.saturating_sub(4)..(peak + 5).min(BINS);
            let weight: f32 = row[window.clone()].iter().sum();
            let cents: f32 = window
                .map(|i| row[i] * (FIRST_BIN_CENTS + 20.0 * i as f32))
                .sum::<f32>()
                / weight;
            PitchFrame::Voiced {
                midi: hz_to_midi(10.0 * 2f32.powf(cents / 1200.0)),
            }
        })
        .collect();
    PitchTrack {
        hop_seconds: HOP as f64 / SAMPLE_RATE as f64,
        frames,
    }
}

/// librosa `filters.mel(sr=16000, n_fft=1024, n_mels=128, fmin=30, fmax=8000, htk=True)`
/// with its default Slaney area normalisation: one row of `N_FFT / 2 + 1` weights per band.
fn mel_filters() -> Vec<Vec<f32>> {
    let to_mel = |hz: f32| 2595.0 * (1.0 + hz / 700.0).log10();
    let to_hz = |mel: f32| 700.0 * (10f32.powf(mel / 2595.0) - 1.0);
    let (lo, hi) = (to_mel(FMIN), to_mel(FMAX));
    let edges: Vec<f32> = (0..N_MELS + 2)
        .map(|i| to_hz(lo + (hi - lo) * i as f32 / (N_MELS + 1) as f32))
        .collect();
    let bins = N_FFT / 2 + 1;
    let bin_hz = |k: usize| k as f32 * SAMPLE_RATE as f32 / N_FFT as f32;
    (0..N_MELS)
        .map(|m| {
            let (left, center, right) = (edges[m], edges[m + 1], edges[m + 2]);
            let norm = 2.0 / (right - left);
            (0..bins)
                .map(|k| {
                    let f = bin_hz(k);
                    let rising = (f - left) / (center - left);
                    let falling = (right - f) / (right - center);
                    rising.min(falling).max(0.0) * norm
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn features_pad_to_a_multiple_of_32_frames() {
        let input = features(&vec![0.0; 16_000]);
        assert_eq!(input.frames, 16_000 / HOP + 1);
        assert_eq!(input.padded, 128);
        assert_eq!(input.mel.len(), N_MELS * 128);
        // Silence sits at the log floor; padding stays zero.
        assert!((input.mel[0] - LOG_FLOOR.ln()).abs() < 1e-6);
        assert_eq!(input.mel[127], 0.0);
    }

    #[test]
    fn mel_filters_cover_the_band_with_unit_area() {
        let filters = mel_filters();
        assert_eq!(filters.len(), N_MELS);
        for f in &filters {
            assert!(f.iter().any(|w| *w > 0.0), "every band has support");
        }
        // Slaney normalisation: each triangle's area (in Hz) is 1.
        let df = SAMPLE_RATE as f32 / N_FFT as f32;
        let area: f32 = filters[64].iter().sum::<f32>() * df;
        assert!((area - 1.0).abs() < 0.1, "area {area}");
    }

    /// A one-hot salience at bin b decodes to that bin's pitch; flat low salience is unvoiced.
    #[test]
    fn decode_reads_bins_as_cents() {
        let mut salience = vec![0.0f32; 2 * BINS];
        salience[100] = 0.9;
        let track = decode(&salience, 2);
        let PitchFrame::Voiced { midi } = track.frames[0] else {
            panic!("frame 0 is voiced")
        };
        let hz = 10.0 * 2f32.powf((FIRST_BIN_CENTS + 2000.0) / 1200.0);
        assert!((midi - hz_to_midi(hz)).abs() < 1e-3);
        assert_eq!(track.frames[1], PitchFrame::Unvoiced);
        assert!((track.hop_seconds - 0.01).abs() < 1e-12);
    }
}
