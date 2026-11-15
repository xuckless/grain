//! Drum transcription (AGENTS.md §4.3): onsets of the drum stem, each classified as kick, snare
//! or closed hi-hat by where its energy sits in the spectrum. A simple, model-free classifier:
//! good on separated drum stems with clear hits, blind to toms, cymbals and simultaneous hits
//! (each onset gets one class).

use crate::midi::MidiNote;
use crate::onset;
use crate::stft::Stft;

pub const KICK: u8 = 36;
pub const SNARE: u8 = 38;
pub const HAT: u8 = 42;

/// Analysis window after each onset.
const N_FFT: usize = 2048;
/// Shortest gap between two hits, seconds.
const MIN_GAP_SECONDS: f64 = 0.05;
/// An onset counts when it is this share of the loudest one…
const ABSOLUTE_THRESHOLD: f32 = 0.12;
/// …and this many times the local average (±`LOCAL_SECONDS`).
const LOCAL_FACTOR: f32 = 1.5;
const LOCAL_SECONDS: f64 = 0.5;
/// Band edges, Hz.
const LOW_HZ: f32 = 150.0;
const HIGH_HZ: f32 = 5_000.0;
/// Class thresholds on energy shares.
const KICK_LOW_SHARE: f32 = 0.4;
const HAT_HIGH_SHARE: f32 = 0.6;
/// Note length written for each hit, seconds.
const HIT_SECONDS: f64 = 0.1;

/// Kick/snare/hat hits in mono `signal`.
pub fn transcribe(signal: &[f32], sample_rate: u32) -> Vec<MidiNote> {
    let env = onset::envelope(signal, sample_rate);
    let v = &env.values;
    let peak = v.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return Vec::new();
    }
    let half = (LOCAL_SECONDS / env.hop_seconds).round() as usize;
    let stft = Stft::new(N_FFT, N_FFT / 2);
    let mut hits: Vec<MidiNote> = Vec::new();
    for i in 1..v.len().saturating_sub(1) {
        if !(v[i] > v[i - 1] && v[i] >= v[i + 1] && v[i] >= ABSOLUTE_THRESHOLD * peak) {
            continue;
        }
        let (a, b) = (i.saturating_sub(half), (i + half + 1).min(v.len()));
        let local = v[a..b].iter().sum::<f32>() / (b - a) as f32;
        if v[i] < LOCAL_FACTOR * local {
            continue;
        }
        let t = i as f64 * env.hop_seconds;
        if hits.last().is_some_and(|h| t - h.start < MIN_GAP_SECONDS) {
            continue;
        }
        let start = ((t - 0.01).max(0.0) * f64::from(sample_rate)) as usize;
        let Some(pitch) = classify(&stft, signal, start, sample_rate) else {
            continue;
        };
        hits.push(MidiNote {
            start: t,
            end: t + HIT_SECONDS,
            pitch,
            velocity: (40.0 + 87.0 * (v[i] / peak).min(1.0)).round() as u8,
        });
    }
    hits
}

/// The class of the hit starting at sample `start`, from its low/high energy shares.
fn classify(stft: &Stft, signal: &[f32], start: usize, sample_rate: u32) -> Option<u8> {
    let end = (start + N_FFT).min(signal.len());
    if end <= start {
        return None;
    }
    let mut window = signal[start..end].to_vec();
    window.resize(N_FFT, 0.0);
    // Frame 1 of a hop-N/2 transform is centred on the window.
    let spectrum = stft.forward(&window).into_iter().nth(1)?;
    let (mut low, mut high, mut total) = (0.0f32, 0.0f32, 0.0f32);
    for (k, c) in spectrum.iter().enumerate().skip(1) {
        let e = c.norm_sqr();
        let hz = stft.bin_hz(k, sample_rate);
        total += e;
        if hz < LOW_HZ {
            low += e;
        } else if hz >= HIGH_HZ {
            high += e;
        }
    }
    if total <= 0.0 {
        return None;
    }
    Some(if low / total >= KICK_LOW_SHARE {
        KICK
    } else if high / total >= HAT_HIGH_SHARE {
        HAT
    } else {
        SNARE
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 44_100;

    /// Deterministic white noise.
    fn noise(n: usize, seed: &mut u32) -> Vec<f32> {
        (0..n)
            .map(|_| {
                *seed ^= *seed << 13;
                *seed ^= *seed >> 17;
                *seed ^= *seed << 5;
                (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }

    fn kick(n: usize) -> Vec<f32> {
        let mut phase = 0.0f32;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                phase += 2.0 * std::f32::consts::PI * (50.0 + 70.0 * (-t * 30.0).exp()) / SR as f32;
                0.9 * phase.sin() * (-t * 18.0).exp()
            })
            .collect()
    }

    fn snare(n: usize, seed: &mut u32) -> Vec<f32> {
        let body = noise(n, seed);
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                (0.7 * (2.0 * std::f32::consts::PI * 200.0 * t).sin() + 0.25 * body[i])
                    * (-t * 25.0).exp()
            })
            .collect()
    }

    fn hat(n: usize, seed: &mut u32) -> Vec<f32> {
        let white = noise(n + 2, seed);
        // A second difference tilts white noise to the top of the spectrum.
        (0..n)
            .map(|i| {
                0.3 * (white[i + 2] - 2.0 * white[i + 1] + white[i])
                    * (-(i as f32 / SR as f32) * 60.0).exp()
            })
            .collect()
    }

    #[test]
    fn classifies_synthetic_kicks_snares_and_hats() {
        let mut seed = 0x1234_5678;
        let pattern = [KICK, HAT, SNARE, HAT, KICK, KICK, SNARE, HAT];
        let step = (0.25 * SR as f64) as usize;
        let mut signal = vec![0.0f32; step * (pattern.len() + 2)];
        for (k, &p) in pattern.iter().enumerate() {
            let hit = match p {
                KICK => kick(step),
                SNARE => snare(step, &mut seed),
                _ => hat(step, &mut seed),
            };
            let at = step * (k + 1);
            for (j, s) in hit.iter().enumerate() {
                signal[at + j] += s;
            }
        }
        let hits = transcribe(&signal, SR);
        let got: Vec<u8> = hits.iter().map(|h| h.pitch).collect();
        assert_eq!(got, pattern, "{hits:?}");
        for (k, h) in hits.iter().enumerate() {
            let want = 0.25 * (k + 1) as f64;
            assert!(
                (h.start - want).abs() < 0.03,
                "hit {k} at {} vs {want}",
                h.start
            );
        }
    }

    #[test]
    fn silence_has_no_hits() {
        assert!(transcribe(&vec![0.0; SR as usize], SR).is_empty());
    }
}
