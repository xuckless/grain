//! Tempo estimation and beat tracking (Ellis 2007 dynamic programming).

use crate::onset::OnsetEnvelope;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

const MIN_BPM: f64 = 60.0;
const MAX_BPM: f64 = 200.0;
/// Center of the log-Gaussian tempo prior, and its width in octaves.
const PRIOR_BPM: f64 = 120.0;
const PRIOR_OCTAVES: f64 = 1.0;
/// Weight of the beat-interval consistency penalty in the beat tracker.
const TIGHTNESS: f32 = 100.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Tempo {
    pub bpm: f64,
    /// Beat times in seconds.
    pub beats: Vec<f64>,
}

pub fn estimate(env: &OnsetEnvelope) -> Tempo {
    let fps = 1.0 / env.hop_seconds;
    let v = &env.values;
    let lag_for = |bpm: f64| (60.0 * fps / bpm).round() as usize;
    let (min_lag, max_lag) = (lag_for(MAX_BPM).max(1), lag_for(MIN_BPM));
    if v.len() <= max_lag + 1 {
        return Tempo {
            bpm: PRIOR_BPM,
            beats: Vec::new(),
        };
    }

    // Autocorrelation weighted by the tempo prior.
    let best_lag = (min_lag..=max_lag)
        .map(|lag| {
            let ac: f32 = v[lag..].iter().zip(v).map(|(a, b)| a * b).sum();
            let bpm = 60.0 * fps / lag as f64;
            let prior = (-0.5 * ((bpm / PRIOR_BPM).log2() / PRIOR_OCTAVES).powi(2)).exp();
            (lag, ac as f64 * prior)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(lag, _)| lag)
        .unwrap_or(lag_for(PRIOR_BPM));
    let period = best_lag as f32;

    // DP: score[t] = onset[t] + max_{p in [t-2P, t-P/2]} score[p] - tightness * log(gap/P)^2
    let n = v.len();
    let mut score = vec![0.0f32; n];
    let mut back = vec![usize::MAX; n];
    for t in 0..n {
        let lo = t.saturating_sub(2 * best_lag);
        let hi = t.saturating_sub(best_lag / 2);
        let mut best = (0.0f32, usize::MAX);
        for (p, prev) in score.iter().enumerate().take(hi).skip(lo) {
            let gap = (t - p) as f32;
            let s = prev - TIGHTNESS * (gap / period).ln().powi(2);
            if best.1 == usize::MAX || s > best.0 {
                best = (s, p);
            }
        }
        score[t] = v[t] + best.0.max(0.0);
        back[t] = if best.0 > 0.0 { best.1 } else { usize::MAX };
    }

    // Backtrack from the best-scoring frame in the final period.
    let tail = n.saturating_sub(best_lag);
    let mut t = (tail..n)
        .max_by(|a, b| score[*a].total_cmp(&score[*b]))
        .unwrap_or(n - 1);
    let mut beats = vec![t];
    while back[t] != usize::MAX {
        t = back[t];
        beats.push(t);
    }
    beats.reverse();
    Tempo {
        bpm: 60.0 * fps / best_lag as f64,
        beats: beats
            .into_iter()
            .map(|b| b as f64 * env.hop_seconds)
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onset;

    /// Clicks on every beat; every second beat (the backbeat) is `accent` times louder.
    fn click_track(bpm: f64, seconds: f64, sr: u32, accent: f32) -> Vec<f32> {
        let mut x = vec![0.0f32; (seconds * sr as f64) as usize];
        let period = (60.0 / bpm * sr as f64) as usize;
        for (beat, start) in (0..x.len()).step_by(period).enumerate() {
            let gain = if beat % 2 == 1 { accent } else { 1.0 };
            for i in 0..400.min(x.len() - start) {
                // Short decaying noise-like burst.
                x[start + i] =
                    gain * ((i as f32 * 1.7).sin() * 13.3).fract() * (-(i as f32) / 80.0).exp();
            }
        }
        x
    }

    #[test]
    fn kick_on_every_beat_with_snare_backbeat_is_not_halved() {
        let sr = 44_100u32;
        let bpm = 120.0;
        let period = (60.0 / bpm * sr as f64) as usize;
        let mut x = vec![0.0f32; 12 * sr as usize];
        for (beat, start) in (0..x.len()).step_by(period).enumerate() {
            for i in 0..4000.min(x.len() - start) {
                let t = i as f32 / sr as f32;
                let kick =
                    (2.0 * std::f32::consts::PI * 60.0 * t * (1.0 + 2.0 * (-t * 40.0).exp())).sin();
                let noise = ((i as f32 * 1.7).sin() * 13.3).fract() - 0.5;
                let snare = if beat % 2 == 1 {
                    0.5 * noise * (-t * 30.0).exp()
                } else {
                    0.0
                };
                x[start + i] += 0.6 * kick * (-t * 18.0).exp() + snare;
            }
        }
        let tempo = estimate(&onset::envelope(&x, sr));
        assert!((tempo.bpm - bpm).abs() / bpm < 0.03, "got {}", tempo.bpm);
    }

    #[test]
    fn detects_click_track_tempo() {
        let sr = 44_100;
        for (bpm, accent) in [
            (90.0, 1.0),
            (120.0, 1.0),
            (140.0, 1.0),
            (120.0, 3.0),
            (100.0, 3.0),
        ] {
            let env = onset::envelope(&click_track(bpm, 12.0, sr, accent), sr);
            let tempo = estimate(&env);
            assert!(
                (tempo.bpm - bpm).abs() / bpm < 0.03,
                "{bpm}: got {}",
                tempo.bpm
            );
            let intervals: Vec<f64> = tempo.beats.windows(2).map(|w| w[1] - w[0]).collect();
            let mean = intervals.iter().sum::<f64>() / intervals.len() as f64;
            assert!((mean - 60.0 / bpm).abs() < 0.02);
        }
    }
}
