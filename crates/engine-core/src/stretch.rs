//! Time warping for the pitch editor's note moves and stretches (AGENTS.md §7.1 step 4).
//!
//! WSOLA (waveform-similarity overlap-add): output frames are read from the source position the
//! [`TimeMap`] gives, nudged by up to a quarter frame to the offset that best continues the
//! previous frame's waveform, and overlap-added with a Hann window. For a monophonic voice this
//! keeps pitch and timbre while changing timing. Only the stretch of the song the map actually
//! changes is processed; everything else is copied bit-for-bit, with short crossfades at the
//! seams.

/// Analysis frame (~23 ms) and its overlap; the search reaches a quarter frame either way.
const FRAME_SECONDS: f64 = 0.023;
/// Unchanged audio kept around the warped stretch, to crossfade into.
const MARGIN_SECONDS: f64 = 0.040;
const CROSSFADE_SECONDS: f64 = 0.010;

/// A monotonic, piecewise-linear map from output time to source time (seconds), through
/// `(output, source)` anchors. Identity before the first anchor and after the last.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeMap {
    anchors: Vec<(f64, f64)>,
}

impl TimeMap {
    /// `anchors`: `(output, source)` pairs; sorted by output time here. Both coordinates must
    /// increase together (callers keep notes from overlapping).
    pub fn new(mut anchors: Vec<(f64, f64)>) -> Self {
        anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { anchors }
    }

    /// Source time heard at output time `t` (callers anchor the song's ends, so outside the
    /// anchors is the identity).
    pub fn source(&self, t: f64) -> f64 {
        let a = &self.anchors;
        let i = a.partition_point(|p| p.0 <= t);
        let (Some(prev), Some(next)) = (i.checked_sub(1).map(|j| a[j]), a.get(i).copied()) else {
            return t;
        };
        let span = next.0 - prev.0;
        if span <= 0.0 {
            return prev.1;
        }
        prev.1 + (t - prev.0) / span * (next.1 - prev.1)
    }

    /// The output-time stretch the map changes, if any: from the anchor before the first moved
    /// anchor to the anchor after the last one.
    pub fn changed(&self) -> Option<(f64, f64)> {
        let a = &self.anchors;
        let moved: Vec<usize> = (0..a.len())
            .filter(|&i| (a[i].0 - a[i].1).abs() > 1e-6)
            .collect();
        let (&first, &last) = (moved.first()?, moved.last()?);
        let lo = first.checked_sub(1).map_or(0.0, |i| a[i].0.min(a[i].1));
        let hi = a
            .get(last + 1)
            .map_or(a[last].0.max(a[last].1), |p| p.0.max(p.1));
        Some((lo.min(a[first].0), hi.max(a[last].0)))
    }
}

/// `signal` re-timed by `map` (output length = input length). Identity maps return a copy.
pub fn warp(signal: &[f32], sample_rate: u32, map: &TimeMap) -> Vec<f32> {
    let mut out = signal.to_vec();
    let Some((lo, hi)) = map.changed() else {
        return out;
    };
    let sr = f64::from(sample_rate);
    let n = ((FRAME_SECONDS * sr) as usize / 2) * 2;
    let hop = n / 2;
    let search = n / 4;
    let window: Vec<f32> = (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
        .collect();
    let margin = (MARGIN_SECONDS * sr) as usize;
    let start = ((lo * sr) as usize).saturating_sub(margin);
    let end = (((hi * sr) as usize) + margin).min(signal.len());
    if end <= start + n {
        return out;
    }
    let len = end - start;
    let sample = |i: i64| -> f32 {
        usize::try_from(i)
            .ok()
            .and_then(|i| signal.get(i).copied())
            .unwrap_or(0.0)
    };

    // Overlap-add output frames centred every `hop` samples across the stretch.
    let mut warped = vec![0.0f32; len + n];
    let mut previous: Option<i64> = None;
    let frames = len / hop + 2;
    for k in 0..frames {
        let centre = start + k * hop;
        let nominal = (map.source(centre as f64 / sr) * sr).round() as i64 - (n / 2) as i64;
        let chosen = match previous {
            None => nominal,
            Some(prev) => {
                let natural = prev + hop as i64;
                best_offset(nominal, natural, search as i64, n, &sample)
            }
        };
        previous = Some(chosen);
        let at = (k * hop) as i64 - (n / 2) as i64;
        for (j, w) in window.iter().enumerate() {
            if let Ok(o) = usize::try_from(at + j as i64) {
                if let Some(v) = warped.get_mut(o) {
                    *v += w * sample(chosen + j as i64);
                }
            }
        }
    }

    // Splice back with crossfades at both seams (the map is the identity there).
    let fade = ((CROSSFADE_SECONDS * sr) as usize).min(len / 2).max(1);
    for i in 0..len {
        let w = if i < fade {
            i as f32 / fade as f32
        } else if i >= len - fade {
            (len - i) as f32 / fade as f32
        } else {
            1.0
        };
        out[start + i] = (1.0 - w) * signal[start + i] + w * warped[i];
    }
    out
}

/// The frame start within `±search` of `nominal` whose waveform best continues `natural`
/// (normalized cross-correlation, every other lag).
fn best_offset(
    nominal: i64,
    natural: i64,
    search: i64,
    n: usize,
    sample: &impl Fn(i64) -> f32,
) -> i64 {
    let reference: Vec<f32> = (0..n as i64)
        .step_by(2)
        .map(|j| sample(natural + j))
        .collect();
    let mut best = (f32::MIN, nominal);
    for lag in (-search..=search).step_by(2) {
        let candidate = nominal + lag;
        let (mut dot, mut energy) = (0.0f32, 1e-9f32);
        for (r, j) in reference.iter().zip((0..n as i64).step_by(2)) {
            let c = sample(candidate + j);
            dot += r * c;
            energy += c * c;
        }
        let score = dot / energy.sqrt();
        if score > best.0 {
            best = (score, candidate);
        }
    }
    best.1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::{self, PitchFrame};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 44_100;

    fn tone(hz: f32, seconds: f64) -> Vec<f32> {
        (0..(seconds * f64::from(SR)) as usize)
            .map(|i| 0.4 * (2.0 * std::f32::consts::PI * hz * i as f32 / SR as f32).sin())
            .collect()
    }

    #[test]
    fn identity_map_is_a_bit_exact_copy() {
        let s = tone(220.0, 0.5);
        let map = TimeMap::new(vec![(0.1, 0.1), (0.3, 0.3)]);
        assert_eq!(map.changed(), None);
        assert_eq!(warp(&s, SR, &map), s);
    }

    #[test]
    fn source_interpolates_between_anchors_and_is_identity_outside() {
        let map = TimeMap::new(vec![(1.0, 1.0), (2.0, 1.5), (3.0, 3.0)]);
        assert_eq!(map.source(0.5), 0.5);
        assert_eq!(map.source(1.0), 1.0);
        assert!((map.source(1.5) - 1.25).abs() < 1e-12);
        assert!((map.source(2.5) - 2.25).abs() < 1e-12);
        assert_eq!(map.source(4.0), 4.0);
        assert_eq!(map.changed(), Some((1.0, 3.0)));
    }

    #[test]
    fn warping_keeps_pitch_and_leaves_the_rest_untouched() {
        // A 1 s tone; the stretch 0.4–0.6 s is played twice as slowly (0.4–0.8 s output).
        let s = tone(220.0, 1.2);
        let map = TimeMap::new(vec![(0.4, 0.4), (0.8, 0.6), (1.0, 1.0)]);
        let w = warp(&s, SR, &map);
        assert_eq!(w.len(), s.len());
        // Untouched before the warped stretch (minus the margin).
        let before = ((0.4 - MARGIN_SECONDS) * f64::from(SR)) as usize - 10;
        assert_eq!(&w[..before], &s[..before]);
        // Pitch is still 220 Hz in the middle of the stretched part.
        let track = pitch::track(&w, SR);
        let Some(PitchFrame::Voiced { midi }) = track.frame_at(0.6).copied() else {
            panic!("unvoiced");
        };
        let expected = crate::note::hz_to_midi(220.0);
        assert!(
            (midi - expected).abs() * 100.0 < 5.0,
            "{midi} vs {expected}"
        );
    }
}
