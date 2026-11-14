//! Onset strength envelope: half-wave rectified flux of log band energies.
//!
//! Bands are log-spaced (like a mel spectrogram) so a narrowband kick drum counts as much as a
//! broadband snare.

use crate::stft::Stft;

const N_FFT: usize = 2048;
const HOP: usize = 512;
const BANDS: usize = 40;
const MIN_HZ: f32 = 30.0;
const MAX_HZ: f32 = 11_000.0;

/// Onset strength sampled every `hop_seconds`.
#[derive(Debug, Clone, PartialEq)]
pub struct OnsetEnvelope {
    pub hop_seconds: f64,
    pub values: Vec<f32>,
}

pub fn envelope(signal: &[f32], sample_rate: u32) -> OnsetEnvelope {
    let stft = Stft::new(N_FFT, HOP);
    let band_of: Vec<Option<usize>> = (0..stft.bins())
        .map(|k| {
            let hz = stft.bin_hz(k, sample_rate);
            (MIN_HZ..MAX_HZ).contains(&hz).then(|| {
                let pos = (hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln();
                ((pos * BANDS as f32) as usize).min(BANDS - 1)
            })
        })
        .collect();
    let log_bands: Vec<[f32; BANDS]> = stft
        .forward(signal)
        .iter()
        .map(|frame| {
            let mut bands = [0.0f32; BANDS];
            for (c, band) in frame.iter().zip(&band_of) {
                if let Some(b) = band {
                    bands[*b] += c.norm_sqr();
                }
            }
            bands.map(|e| (1.0 + 1000.0 * e).ln())
        })
        .collect();
    let mut values = vec![0.0f32; log_bands.len()];
    for t in 1..log_bands.len() {
        values[t] = log_bands[t]
            .iter()
            .zip(&log_bands[t - 1])
            .map(|(a, b)| (a - b).max(0.0))
            .sum::<f32>()
            / BANDS as f32;
    }
    // Remove the average level so sustained sound does not count as onsets.
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    values.iter_mut().for_each(|v| *v = (*v - mean).max(0.0));
    OnsetEnvelope {
        hop_seconds: HOP as f64 / sample_rate as f64,
        values,
    }
}
