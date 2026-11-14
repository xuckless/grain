//! Onset strength envelope (half-wave rectified log-spectral flux).

use crate::stft::Stft;

const N_FFT: usize = 2048;
const HOP: usize = 512;

/// Onset strength sampled every `hop_seconds`.
#[derive(Debug, Clone, PartialEq)]
pub struct OnsetEnvelope {
    pub hop_seconds: f64,
    pub values: Vec<f32>,
}

pub fn envelope(signal: &[f32], sample_rate: u32) -> OnsetEnvelope {
    let stft = Stft::new(N_FFT, HOP);
    let frames = stft.forward(signal);
    let log_mag: Vec<Vec<f32>> = frames
        .iter()
        .map(|f| f.iter().map(|c| (1.0 + 100.0 * c.norm()).ln()).collect())
        .collect();
    let mut values = vec![0.0f32; log_mag.len()];
    for t in 1..log_mag.len() {
        values[t] = log_mag[t]
            .iter()
            .zip(&log_mag[t - 1])
            .map(|(a, b)| (a - b).max(0.0))
            .sum();
    }
    // Remove the slowly varying baseline so silence-to-sound level changes do not dominate.
    let mean = values.iter().sum::<f32>() / values.len().max(1) as f32;
    values.iter_mut().for_each(|v| *v = (*v - mean).max(0.0));
    OnsetEnvelope {
        hop_seconds: HOP as f64 / sample_rate as f64,
        values,
    }
}
