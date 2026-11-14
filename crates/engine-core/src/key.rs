//! Key detection: chroma correlated against Krumhansl–Schmuckler profiles (AGENTS.md §4.4).

use crate::note::hz_to_midi;
use crate::stft::Stft;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const N_FFT: usize = 8192;
const HOP: usize = 4096;
const MIN_HZ: f32 = 55.0;
const MAX_HZ: f32 = 2000.0;

const MAJOR_PROFILE: [f32; 12] = [
    6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88,
];
const MINOR_PROFILE: [f32; 12] = [
    6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17,
];
const MAJOR_SCALE: [u8; 7] = [0, 2, 4, 5, 7, 9, 11];
const MINOR_SCALE: [u8; 7] = [0, 2, 3, 5, 7, 8, 10];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Mode {
    Major,
    Minor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Key {
    /// Pitch class of the tonic, 0 = C.
    pub tonic: u8,
    pub mode: Mode,
}

impl Key {
    pub fn contains(&self, pitch_class: u8) -> bool {
        let scale = match self.mode {
            Mode::Major => &MAJOR_SCALE,
            Mode::Minor => &MINOR_SCALE,
        };
        scale.contains(&((pitch_class + 12 - self.tonic) % 12))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct KeyCandidate {
    pub key: Key,
    /// Pearson correlation between the song's chroma and the key profile.
    pub correlation: f32,
}

/// Energy per pitch class over the whole signal.
pub fn chroma(signal: &[f32], sample_rate: u32) -> [f32; 12] {
    let stft = Stft::new(N_FFT, HOP);
    let classes: Vec<Option<usize>> = (0..stft.bins())
        .map(|k| {
            let hz = stft.bin_hz(k, sample_rate);
            (MIN_HZ..=MAX_HZ)
                .contains(&hz)
                .then(|| (hz_to_midi(hz).round() as i32).rem_euclid(12) as usize)
        })
        .collect();
    let mut chroma = [0.0f32; 12];
    for frame in stft.forward(signal) {
        for (c, pc) in frame.iter().zip(&classes) {
            if let Some(pc) = pc {
                chroma[*pc] += c.norm_sqr();
            }
        }
    }
    chroma
}

/// The three best-matching keys, best first.
pub fn estimate(signal: &[f32], sample_rate: u32) -> Vec<KeyCandidate> {
    let chroma = chroma(signal, sample_rate);
    let mut candidates: Vec<KeyCandidate> = [Mode::Major, Mode::Minor]
        .into_iter()
        .flat_map(|mode| {
            let profile = match mode {
                Mode::Major => &MAJOR_PROFILE,
                Mode::Minor => &MINOR_PROFILE,
            };
            (0..12u8).map(move |tonic| {
                let rotated: Vec<f32> = (0..12)
                    .map(|i| profile[(i + 12 - tonic as usize) % 12])
                    .collect();
                KeyCandidate {
                    key: Key { tonic, mode },
                    correlation: pearson(&chroma, &rotated),
                }
            })
        })
        .collect();
    candidates.sort_by(|a, b| b.correlation.total_cmp(&a.correlation));
    candidates.truncate(3);
    candidates
}

fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let cov: f32 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let va: f32 = a.iter().map(|x| (x - ma).powi(2)).sum();
    let vb: f32 = b.iter().map(|y| (y - mb).powi(2)).sum();
    cov / (va.sqrt() * vb.sqrt() + 1e-12)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::midi_to_hz;
    use crate::pitch::tests::harmonic;

    #[test]
    fn detects_key_of_a_scale_with_tonic_emphasis() {
        let sr = 44_100;
        // D major: tonic triad weighted, then the full scale.
        let mut x = Vec::new();
        for midi in [62, 66, 69, 62, 64, 66, 67, 69, 71, 73, 74, 62, 69] {
            x.extend(harmonic(midi_to_hz(midi as f32), 0.4, sr));
        }
        let best = estimate(&x, sr)[0];
        assert_eq!(
            best.key,
            Key {
                tonic: 2,
                mode: Mode::Major
            }
        );
    }

    #[test]
    fn scale_membership() {
        let a_minor = Key {
            tonic: 9,
            mode: Mode::Minor,
        };
        assert!(a_minor.contains(0) && a_minor.contains(7) && !a_minor.contains(1));
    }
}
